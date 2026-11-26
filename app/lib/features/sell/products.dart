import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/format/money.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import '../catalog/catalog_models.dart';
import '../catalog/catalog_repository.dart';
import 'listing_models.dart';
import 'photos/camera_page.dart';
import 'photos/photo_prep.dart';
import 'photos/photo_section.dart';
import 'photos/photo_source.dart';
import 'photos/photo_upload.dart';
import 'sell_models.dart';
import 'sell_repository.dart';
import 'sell_strings.dart';
import 'sell_widgets.dart';

final listingsProvider = FutureProvider.autoDispose
    .family<List<Listing>, String>(
      (ref, storeId) => ref.watch(workRepositoryProvider).listings(storeId),
    );

final categoriesProvider = FutureProvider.autoDispose<List<Category>>(
  (ref) => ref.watch(catalogRepositoryProvider).categories(),
);

final branchesProvider = FutureProvider.autoDispose
    .family<List<Branch>, String>(
      (ref, storeId) => ref.watch(sellRepositoryProvider).branches(storeId),
    );

/// The Products tab: the store's products and a way to add one.
class ProductsTab extends ConsumerWidget {
  const ProductsTab({super.key, required this.store});

  final SellStore store;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final listings = ref.watch(listingsProvider(store.id));
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: sellRootAppBar(context, SellStrings.tabProducts),
      floatingActionButton: FloatingActionButton.extended(
        onPressed: () async {
          await pushPage<void>(context, ProductFormPage(store: store));
          ref.invalidate(listingsProvider(store.id));
        },
        icon: const Icon(Icons.add),
        label: const Text(SellStrings.addProduct),
      ),
      body: AsyncBody<List<Listing>>(
        value: listings,
        onRetry: () => ref.invalidate(listingsProvider(store.id)),
        builder: (items) => items.isEmpty
            ? const EmptyState(
                title: SellStrings.noProducts,
                body: SellStrings.noProductsBody,
              )
            : RefreshIndicator(
                onRefresh: () => ref.refresh(listingsProvider(store.id).future),
                child: ListView.separated(
                  padding: const EdgeInsets.only(bottom: 96),
                  itemCount: items.length,
                  separatorBuilder: (_, _) => const Divider(height: 1),
                  itemBuilder: (context, i) {
                    final p = items[i];
                    return SellRow(
                      title: p.name,
                      subtitle:
                          '${formatNaira(p.basePriceKobo)}  ·  ${SellStrings.stockLine(p.totalStock)}',
                      trailing: StatusPill(
                        label: p.published
                            ? SellStrings.published
                            : SellStrings.hidden,
                        good: p.published,
                      ),
                      onTap: () async {
                        await pushPage<void>(
                          context,
                          ProductFormPage(store: store, listing: p),
                        );
                        ref.invalidate(listingsProvider(store.id));
                      },
                    );
                  },
                ),
              ),
      ),
    );
  }
}

class _VariantRow {
  _VariantRow({String label = '', Map<String, int>? stock, this.existingId})
    : label = TextEditingController(text: label),
      stock = {
        for (final e in (stock ?? const <String, int>{}).entries)
          e.key: TextEditingController(text: '${e.value}'),
      };

  final String? existingId;
  final TextEditingController label;

  /// Branch id to the count's field.
  final Map<String, TextEditingController> stock;

  void dispose() {
    label.dispose();
    for (final c in stock.values) {
      c.dispose();
    }
  }
}

/// Create or edit a product: name, price, category, photos and the stock of each
/// size at each branch the person may edit.
class ProductFormPage extends ConsumerStatefulWidget {
  const ProductFormPage({super.key, required this.store, this.listing});

  final SellStore store;
  final Listing? listing;

  @override
  ConsumerState<ProductFormPage> createState() => _ProductFormPageState();
}

class _ProductFormPageState extends ConsumerState<ProductFormPage>
    with BusyAction {
  final _name = TextEditingController();
  final _about = TextEditingController();
  final _price = TextEditingController();
  String? _category;
  String _condition = 'new';
  final List<PhotoEntry> _photos = [];
  final List<_VariantRow> _variants = [];
  List<Branch> _branches = const [];
  bool _ready = false;
  bool _published = false;

  Listing? get _editing => widget.listing;
  StoreRole get _role => widget.store.role;

  @override
  void initState() {
    super.initState();
    final l = _editing;
    if (l != null) {
      _name.text = l.name;
      _about.text = l.description;
      _price.text = (l.basePriceKobo / 100).toString().replaceAll(
        RegExp(r'\.0$'),
        '',
      );
      _category = l.category;
      _condition = l.condition;
      _published = l.published;
      for (var i = 0; i < l.photoKeys.length; i++) {
        _photos.add(
          PhotoEntry.saved(
            key: l.photoKeys[i],
            url: i < l.photoUrls.length ? l.photoUrls[i] : null,
          ),
        );
      }
    }
    unawaited(_loadBranches());
  }

  Future<void> _loadBranches() async {
    try {
      final all = await ref
          .read(sellRepositoryProvider)
          .branches(widget.store.id);
      var open = [
        for (final b in all)
          if (!b.closed) b,
      ];
      // Staff tied to a branch count stock only there.
      final tied = widget.store.branchId;
      if (_role == StoreRole.staff && tied != null) {
        open = [
          for (final b in open)
            if (b.id == tied) b,
        ];
      }
      if (!mounted) return;
      setState(() {
        _branches = open;
        final l = _editing;
        if (l == null) {
          _variants.add(_VariantRow(stock: {for (final b in open) b.id: 0}));
        } else {
          for (final v in l.variants) {
            _variants.add(
              _VariantRow(
                label: v.label,
                existingId: v.id,
                stock: {
                  for (final b in open) b.id: open.length == 1 ? v.stock : 0,
                },
              ),
            );
          }
        }
        _ready = true;
      });
    } on Object catch (e) {
      if (mounted) {
        setState(() {
          error = sellError(e);
          _ready = true;
        });
      }
    }
  }

  @override
  void dispose() {
    for (final c in [_name, _about, _price]) {
      c.dispose();
    }
    for (final v in _variants) {
      v.dispose();
    }
    super.dispose();
  }

  bool get _uploading => _photos.any((p) => p.status == PhotoStatus.uploading);
  bool get _canSave =>
      _name.text.trim().isNotEmpty &&
      nairaToKobo(_price.text) != null &&
      _category != null &&
      _photos.any((p) => p.status == PhotoStatus.ready) &&
      !_uploading;

  Future<void> _addPhoto(Future<Uint8List?> Function() pick) async {
    final raw = await pick();
    if (raw == null || !mounted) return;
    final entry = PhotoEntry.fresh(raw);
    setState(() => _photos.add(entry));
    unawaited(_upload(entry));
  }

  Future<void> _upload(PhotoEntry entry) async {
    final raw = entry.bytes;
    if (raw == null) return;
    setState(() => entry.status = PhotoStatus.uploading);
    try {
      final prepared = await preparePhoto(raw);
      if (prepared == null) throw const FormatException('not a picture');
      entry.key = await ref.read(photoUploaderProvider).upload(prepared);
      if (mounted) setState(() => entry.status = PhotoStatus.ready);
    } on Object {
      if (mounted) setState(() => entry.status = PhotoStatus.failed);
    }
  }

  Future<void> _guideThenCamera() async {
    final source = ref.read(photoSourceProvider);
    await _addPhoto(
      () => source.fromCamera(context, shotIndex: _photos.length),
    );
  }

  Map<String, int> _counts(_VariantRow v) => {
    for (final e in v.stock.entries)
      e.key: int.tryParse(e.value.text.trim()) ?? 0,
  };

  Future<void> _save() async {
    final price = nairaToKobo(_price.text);
    final category = _category;
    if (!_canSave || price == null || category == null) return;
    final input = ListingInput(
      name: _name.text.trim(),
      description: _about.text.trim(),
      category: category,
      condition: _condition,
      priceKobo: price,
      photoKeys: [
        for (final p in _photos)
          if (p.key != null) p.key!,
      ],
      variants: [
        for (final v in _variants)
          VariantInput(
            attributes: v.label.text.trim().isEmpty
                ? const {}
                : {'option': v.label.text.trim()},
            stock: _counts(v),
          ),
      ],
    );
    final ok = await run(() async {
      final work = ref.read(workRepositoryProvider);
      final l = _editing;
      if (l == null) {
        await work.createListing(
          widget.store.id,
          input,
          singleBranch: _branches.length <= 1,
        );
      } else {
        await work.editListing(l.id, input);
        // Counts at the branches this person may edit.
        for (final b in _branches) {
          final changes = <String, int>{
            for (final v in _variants)
              if (v.existingId != null && v.stock[b.id] != null)
                v.existingId!: _counts(v)[b.id] ?? 0,
          };
          if (changes.isNotEmpty) await work.setStock(b.id, changes);
        }
        if (_role.isManagerUp && _published != l.published) {
          await work.setPublished(l.id, _published);
        }
      }
    });
    if (ok && mounted) Navigator.of(context).pop();
  }

  Future<void> _delete() async {
    final l = _editing;
    if (l == null) return;
    final ok = await run(
      () => ref.read(workRepositoryProvider).deleteListing(l.id),
    );
    if (ok && mounted) Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final categories = ref.watch(categoriesProvider).value ?? const [];
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: sellAppBar(
        context,
        _editing == null ? SellStrings.addProduct : SellStrings.editProduct,
      ),
      body: SafeArea(
        child: !_ready
            ? const Center(child: CircularProgressIndicator())
            : ListView(
                padding: const EdgeInsets.all(Tokens.s5),
                children: [
                  PhotoSection(
                    entries: _photos,
                    onGuide: () => showPhotoGuide(context),
                    onCamera: _guideThenCamera,
                    onGallery: () =>
                        _addPhoto(ref.read(photoSourceProvider).fromGallery),
                    onRemove: (i) => setState(() => _photos.removeAt(i)),
                    onRetry: (i) => _upload(_photos[i]),
                  ),
                  const SizedBox(height: Tokens.s4),
                  AppTextField(
                    label: SellStrings.productName,
                    controller: _name,
                    maxLength: 120,
                    capitalization: TextCapitalization.sentences,
                    onChanged: (_) => setState(() {}),
                  ),
                  const SizedBox(height: Tokens.s3),
                  AppTextField(
                    label: SellStrings.productDescription,
                    controller: _about,
                    maxLines: 4,
                    maxLength: 2000,
                    capitalization: TextCapitalization.sentences,
                    onChanged: (_) => setState(() {}),
                  ),
                  const SizedBox(height: Tokens.s3),
                  DropdownButtonFormField<String>(
                    initialValue: categories.any((c) => c.slug == _category)
                        ? _category
                        : null,
                    decoration: const InputDecoration(
                      labelText: SellStrings.productCategory,
                    ),
                    items: [
                      for (final c in categories)
                        DropdownMenuItem(value: c.slug, child: Text(c.name)),
                    ],
                    onChanged: (v) => setState(() => _category = v),
                  ),
                  const SizedBox(height: Tokens.s3),
                  SegmentedButton<String>(
                    segments: const [
                      ButtonSegment(
                        value: 'new',
                        label: Text(SellStrings.conditionNew),
                      ),
                      ButtonSegment(
                        value: 'used',
                        label: Text(SellStrings.conditionUsed),
                      ),
                    ],
                    selected: {_condition},
                    onSelectionChanged: (s) =>
                        setState(() => _condition = s.first),
                  ),
                  const SizedBox(height: Tokens.s3),
                  AppTextField(
                    label: SellStrings.productPrice,
                    controller: _price,
                    keyboardType: const TextInputType.numberWithOptions(
                      decimal: true,
                    ),
                    formatters: [
                      FilteringTextInputFormatter.allow(RegExp(r'[0-9.,]')),
                    ],
                    onChanged: (_) => setState(() {}),
                  ),
                  const SizedBox(height: Tokens.s4),
                  Text(
                    SellStrings.productVariants,
                    style: theme.textTheme.titleMedium,
                  ),
                  for (final v in _variants) ...[
                    const SizedBox(height: Tokens.s2),
                    AppTextField(
                      label: SellStrings.variantLabel,
                      controller: v.label,
                      maxLength: 40,
                    ),
                    for (final b in _branches)
                      Padding(
                        padding: const EdgeInsets.only(top: Tokens.s2),
                        child: AppTextField(
                          label: _branches.length > 1
                              ? '${SellStrings.variantStock}: ${b.name}'
                              : SellStrings.variantStock,
                          controller: v.stock[b.id],
                          keyboardType: TextInputType.number,
                          maxLength: 6,
                          formatters: [FilteringTextInputFormatter.digitsOnly],
                        ),
                      ),
                  ],
                  if (_editing == null)
                    TextButton.icon(
                      onPressed: () => setState(
                        () => _variants.add(
                          _VariantRow(
                            stock: {for (final b in _branches) b.id: 0},
                          ),
                        ),
                      ),
                      icon: const Icon(Icons.add),
                      label: const Text(SellStrings.addVariant),
                    ),
                  if (_editing != null && _role.isManagerUp)
                    SwitchListTile(
                      contentPadding: EdgeInsets.zero,
                      title: const Text(SellStrings.published),
                      value: _published,
                      onChanged: (v) => setState(() => _published = v),
                    ),
                  if (error != null) ...[
                    const SizedBox(height: Tokens.s3),
                    ErrorLine(error!),
                  ],
                  if (!_canSave && !busy) ...[
                    const SizedBox(height: Tokens.s2),
                    Text(
                      SellStrings.productNeedsDetails,
                      style: theme.textTheme.bodyMedium?.copyWith(
                        color: context.tokens.mute,
                      ),
                    ),
                  ],
                  const SizedBox(height: Tokens.s4),
                  LoadingButton(
                    label: SellStrings.saveProduct,
                    loading: busy,
                    onPressed: _canSave ? _save : null,
                  ),
                  if (_editing != null && _role.isManagerUp) ...[
                    const SizedBox(height: Tokens.s2),
                    TextButton(
                      onPressed: busy ? null : _delete,
                      child: Text(
                        SellStrings.deleteProduct,
                        style: TextStyle(color: context.tokens.errorText),
                      ),
                    ),
                  ],
                ],
              ),
      ),
    );
  }
}
