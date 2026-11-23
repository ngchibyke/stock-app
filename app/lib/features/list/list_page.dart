import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import '../cart/cart_button.dart';
import '../catalog/catalog_models.dart';
import '../catalog/catalog_repository.dart';
import 'paged_list.dart';
import 'rows.dart';

final categoriesProvider = FutureProvider.autoDispose<List<Category>>(
  (ref) => ref.watch(catalogRepositoryProvider).categories(),
);

/// The market as a plain list: search, categories, products and stores.
class ListPage extends ConsumerStatefulWidget {
  const ListPage({super.key, this.focusSearch = false});

  /// Opened from the market's search field: the keyboard comes straight up.
  final bool focusSearch;

  @override
  ConsumerState<ListPage> createState() => _ListPageState();
}

class _ListPageState extends ConsumerState<ListPage> {
  final _search = TextEditingController();
  Timer? _debounce;
  String _query = '';
  String? _category;

  @override
  void dispose() {
    _debounce?.cancel();
    _search.dispose();
    super.dispose();
  }

  void _onChanged(String text) {
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 300), () {
      if (mounted) setState(() => _query = text.trim());
    });
  }

  void _clear() {
    _debounce?.cancel();
    _search.clear();
    setState(() => _query = '');
  }

  @override
  Widget build(BuildContext context) {
    final catalog = ref.watch(catalogRepositoryProvider);
    final tokens = context.tokens;
    final q = _query.isEmpty ? null : _query;

    Widget empty() => _query.isEmpty
        ? const EmptyState(title: Strings.nothingHereYet)
        : EmptyState(
            title: Strings.nothingMatches(_query),
            actionLabel: Strings.clearSearch,
            onAction: _clear,
          );

    return DefaultTabController(
      length: 2,
      child: Scaffold(
        appBar: AppBar(
          titleSpacing: 0,
          title: TextField(
            controller: _search,
            autofocus: widget.focusSearch,
            maxLength: maxSearchLength,
            textInputAction: TextInputAction.search,
            onChanged: _onChanged,
            decoration: InputDecoration(
              hintText: Strings.searchMarket,
              counterText: '',
              border: InputBorder.none,
              filled: false,
              prefixIcon: Icon(Icons.search, color: tokens.mute),
              suffixIcon: ListenableBuilder(
                listenable: _search,
                builder: (context, _) => _search.text.isEmpty
                    ? const SizedBox.shrink()
                    : IconButton(
                        tooltip: Strings.clearSearch,
                        icon: const Icon(Icons.close),
                        onPressed: _clear,
                      ),
              ),
            ),
          ),
          actions: const [
            CartButton(),
            SizedBox(width: Tokens.s2),
          ],
          bottom: const TabBar(
            tabs: [
              Tab(text: Strings.productsTab),
              Tab(text: Strings.storesTab),
            ],
          ),
        ),
        body: TabBarView(
          children: [
            Column(
              children: [
                _Categories(
                  chosen: _category,
                  onChosen: (c) => setState(() => _category = c),
                ),
                Expanded(
                  child: PagedList<ProductCard>(
                    key: ValueKey(('products', q, _category)),
                    load: (cursor) => catalog.products(
                      q: q,
                      category: _category,
                      cursor: cursor,
                    ),
                    itemBuilder: (context, p) => ProductRow(product: p),
                    empty: empty(),
                  ),
                ),
              ],
            ),
            PagedList<StoreSummary>(
              key: ValueKey(('stores', q)),
              load: (cursor) => catalog.stores(q: q, cursor: cursor),
              itemBuilder: (context, s) => StoreRow(store: s),
              empty: empty(),
            ),
          ],
        ),
      ),
    );
  }
}

class _Categories extends ConsumerWidget {
  const _Categories({required this.chosen, required this.onChosen});

  final String? chosen;
  final ValueChanged<String?> onChosen;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final list = ref.watch(categoriesProvider).value ?? const <Category>[];
    if (list.isEmpty) return const SizedBox.shrink();
    return SizedBox(
      height: Tokens.minTap + Tokens.s3,
      child: ListView(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.fromLTRB(Tokens.s4, Tokens.s2, Tokens.s4, 0),
        children: [
          for (final (slug, name) in [
            (null, Strings.allCategories),
            for (final c in list) (c.slug, c.name),
          ])
            Padding(
              padding: const EdgeInsets.only(right: Tokens.s2),
              child: ChoiceChip(
                label: Text(name),
                selected: chosen == slug,
                onSelected: (_) => onChosen(slug),
              ),
            ),
        ],
      ),
    );
  }
}
