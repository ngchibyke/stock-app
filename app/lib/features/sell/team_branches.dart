import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import '../addresses/address.dart';
import 'products.dart' show branchesProvider;
import 'sell_models.dart';
import 'sell_providers.dart';
import 'sell_repository.dart';
import 'sell_strings.dart';
import 'sell_widgets.dart';

final teamProvider = FutureProvider.autoDispose.family<Team, String>(
  (ref, storeId) => ref.watch(sellRepositoryProvider).team(storeId),
);

String roleLabel(StoreRole r) => switch (r) {
  StoreRole.owner => SellStrings.roleOwner,
  StoreRole.manager => SellStrings.roleManager,
  StoreRole.staff => SellStrings.roleStaff,
};

/// The Team tab: members, pending invites and the way to invite. Owners edit;
/// managers read.
class TeamTab extends ConsumerWidget {
  const TeamTab({super.key, required this.store});

  final SellStore store;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final team = ref.watch(teamProvider(store.id));
    final owner = store.role.isOwner;
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: sellRootAppBar(context, SellStrings.teamTitle),
      floatingActionButton: owner
          ? FloatingActionButton.extended(
              onPressed: () async {
                await pushPage<void>(context, InvitePage(store: store));
                ref.invalidate(teamProvider(store.id));
              },
              icon: const Icon(Icons.person_add_alt),
              label: const Text(SellStrings.inviteTitle),
            )
          : null,
      body: AsyncBody<Team>(
        value: team,
        onRetry: () => ref.invalidate(teamProvider(store.id)),
        builder: (t) => ListView(
          padding: const EdgeInsets.only(bottom: 96),
          children: [
            if (t.members.isEmpty && t.invites.isEmpty)
              const EmptyState(
                title: SellStrings.noTeam,
                body: SellStrings.managerNote,
              ),
            for (final m in t.members)
              SellRow(
                title: m.name,
                subtitle:
                    '${roleLabel(m.role)}${m.branch == null ? '' : '  ·  ${m.branch!.name}'}',
                trailing: owner
                    ? TextButton(
                        onPressed: () async {
                          await ref
                              .read(sellRepositoryProvider)
                              .removeMember(store.id, m.userId);
                          ref.invalidate(teamProvider(store.id));
                        },
                        child: const Text(SellStrings.remove),
                      )
                    : null,
              ),
            if (t.invites.isNotEmpty)
              Padding(
                padding: const EdgeInsets.fromLTRB(
                  Tokens.s5,
                  Tokens.s4,
                  Tokens.s5,
                  Tokens.s1,
                ),
                child: Text(
                  SellStrings.pendingInvites,
                  style: Theme.of(context).textTheme.titleSmall,
                ),
              ),
            for (final i in t.invites)
              SellRow(
                title: SellStrings.inviteLine(i.last4),
                subtitle:
                    '${roleLabel(i.role)}${i.branch == null ? '' : '  ·  ${i.branch!.name}'}',
                trailing: owner
                    ? TextButton(
                        onPressed: () async {
                          await ref
                              .read(sellRepositoryProvider)
                              .revokeInvite(store.id, i.id);
                          ref.invalidate(teamProvider(store.id));
                        },
                        child: const Text(SellStrings.revoke),
                      )
                    : null,
              ),
          ],
        ),
      ),
    );
  }
}

/// Invite someone by phone as a manager or as staff (store-wide or tied to a branch).
class InvitePage extends ConsumerStatefulWidget {
  const InvitePage({super.key, required this.store});

  final SellStore store;

  @override
  ConsumerState<InvitePage> createState() => _InvitePageState();
}

class _InvitePageState extends ConsumerState<InvitePage> with BusyAction {
  final _phone = TextEditingController();
  StoreRole _role = StoreRole.staff;
  String? _branch;

  @override
  void dispose() {
    _phone.clear();
    _phone.dispose();
    super.dispose();
  }

  Future<void> _send() async {
    final ok = await run(
      () => ref
          .read(sellRepositoryProvider)
          .invite(
            widget.store.id,
            phone: _phone.text.trim(),
            role: _role,
            branchId: _branch,
          ),
    );
    if (ok && mounted) Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    final branches =
        ref.watch(branchesProvider(widget.store.id)).value ?? const [];
    final open = [
      for (final b in branches)
        if (!b.closed) b,
    ];
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: sellAppBar(context, SellStrings.inviteTitle),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.all(Tokens.s5),
          children: [
            AppTextField(
              label: SellStrings.invitePhone,
              controller: _phone,
              keyboardType: TextInputType.phone,
              maxLength: 14,
              formatters: [
                FilteringTextInputFormatter.allow(RegExp(r'[0-9+]')),
              ],
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: Tokens.s3),
            SegmentedButton<StoreRole>(
              showSelectedIcon: false,
              segments: const [
                ButtonSegment(
                  value: StoreRole.staff,
                  label: Text(SellStrings.roleStaff),
                ),
                ButtonSegment(
                  value: StoreRole.manager,
                  label: Text(SellStrings.roleManager),
                ),
              ],
              selected: {_role},
              onSelectionChanged: (s) => setState(() {
                _role = s.first;
                if (_role == StoreRole.manager) _branch = null;
              }),
            ),
            const SizedBox(height: Tokens.s2),
            Text(
              SellStrings.managerNote,
              style: Theme.of(context).textTheme.bodyMedium,
            ),
            if (_role == StoreRole.staff && open.length > 1) ...[
              const SizedBox(height: Tokens.s3),
              DropdownButtonFormField<String?>(
                initialValue: _branch,
                decoration: const InputDecoration(
                  labelText: SellStrings.tiedTo,
                ),
                items: [
                  const DropdownMenuItem<String?>(
                    value: null,
                    child: Text(SellStrings.allBranches),
                  ),
                  for (final b in open)
                    DropdownMenuItem<String?>(value: b.id, child: Text(b.name)),
                ],
                onChanged: (v) => setState(() => _branch = v),
              ),
            ],
            if (error != null) ...[
              const SizedBox(height: Tokens.s3),
              ErrorLine(error!),
            ],
            const SizedBox(height: Tokens.s4),
            LoadingButton(
              label: SellStrings.sendInvite,
              loading: busy,
              onPressed: _phone.text.trim().length < 10 ? null : _send,
            ),
          ],
        ),
      ),
    );
  }
}

/// Invitations the person has received, with accept and decline.
class InvitesPage extends ConsumerWidget {
  const InvitesPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final invites = ref.watch(myInvitesProvider);
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: sellAppBar(context, SellStrings.invitesTitle),
      body: AsyncBody<List<MyInvite>>(
        value: invites,
        onRetry: () => ref.invalidate(myInvitesProvider),
        builder: (items) => items.isEmpty
            ? const EmptyState(title: SellStrings.noTeam)
            : ListView(
                children: [
                  for (final i in items)
                    SellRow(
                      title: SellStrings.inviteFrom(
                        i.storeName,
                        roleLabel(i.role),
                      ),
                      subtitle: i.branchName,
                      trailing: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          TextButton(
                            onPressed: () async {
                              await ref
                                  .read(sellRepositoryProvider)
                                  .answerInvite(i.id, accept: false);
                              ref.invalidate(myInvitesProvider);
                            },
                            child: const Text(SellStrings.decline),
                          ),
                          FilledButton(
                            onPressed: () async {
                              await ref
                                  .read(sellRepositoryProvider)
                                  .answerInvite(i.id, accept: true);
                              ref.invalidate(myInvitesProvider);
                              ref.invalidate(myStoresProvider);
                            },
                            child: const Text(SellStrings.accept),
                          ),
                        ],
                      ),
                    ),
                ],
              ),
      ),
    );
  }
}

/// A store's branches: list, add, edit, close. Only owners and managers see the
/// details; only owners change them.
class BranchesPage extends ConsumerWidget {
  const BranchesPage({super.key, required this.store});

  final SellStore store;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final branches = ref.watch(branchesProvider(store.id));
    final owner = store.role.isOwner;
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: sellAppBar(context, SellStrings.branchesTitle),
      floatingActionButton: owner
          ? FloatingActionButton.extended(
              onPressed: () async {
                await pushPage<void>(context, BranchFormPage(store: store));
                ref.invalidate(branchesProvider(store.id));
              },
              icon: const Icon(Icons.add),
              label: const Text(SellStrings.addBranch),
            )
          : null,
      body: AsyncBody<List<Branch>>(
        value: branches,
        onRetry: () => ref.invalidate(branchesProvider(store.id)),
        builder: (items) => ListView.separated(
          padding: const EdgeInsets.only(bottom: 96),
          itemCount: items.length,
          separatorBuilder: (_, _) => const Divider(height: 1),
          itemBuilder: (context, i) {
            final b = items[i];
            return SellRow(
              title: b.name,
              subtitle: b.address,
              trailing: b.closed
                  ? const StatusPill(label: SellStrings.closed)
                  : null,
              onTap: owner && !b.closed
                  ? () async {
                      await pushPage<void>(
                        context,
                        BranchFormPage(store: store, branch: b),
                      );
                      ref.invalidate(branchesProvider(store.id));
                    }
                  : null,
            );
          },
        ),
      ),
    );
  }
}

/// Add or edit a branch, including the collect switch with its notice.
class BranchFormPage extends ConsumerStatefulWidget {
  const BranchFormPage({super.key, required this.store, this.branch});

  final SellStore store;
  final Branch? branch;

  @override
  ConsumerState<BranchFormPage> createState() => _BranchFormPageState();
}

class _BranchFormPageState extends ConsumerState<BranchFormPage>
    with BusyAction {
  late final _name = TextEditingController(text: widget.branch?.name ?? '');
  late final _address = TextEditingController(
    text: widget.branch?.address ?? '',
  );
  late final _phone = TextEditingController(text: widget.branch?.phone ?? '');
  late final _fee = TextEditingController(
    text: widget.branch?.feeKobo == null
        ? ''
        : (widget.branch!.feeKobo! / 100).toString().replaceAll(
            RegExp(r'\.0$'),
            '',
          ),
  );
  late AreaCouncil _council =
      AreaCouncil.fromWire(widget.branch?.areaCouncil) ?? AreaCouncil.amac;
  late bool _collect = widget.branch?.collectEnabled ?? false;

  @override
  void dispose() {
    for (final c in [_name, _address, _phone, _fee]) {
      c.dispose();
    }
    super.dispose();
  }

  bool get _ready =>
      _name.text.trim().isNotEmpty &&
      _address.text.trim().isNotEmpty &&
      _phone.text.trim().length >= 10;

  Future<void> _toggleCollect(bool on) async {
    if (!on) {
      setState(() => _collect = false);
      return;
    }
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text(SellStrings.collectSwitch),
        content: const Text(SellStrings.collectNotice),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text(SellStrings.cancel),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text(SellStrings.collectConfirm),
          ),
        ],
      ),
    );
    if (sure == true && mounted) setState(() => _collect = true);
  }

  Future<void> _save() async {
    final input = BranchInput(
      name: _name.text.trim(),
      address: _address.text.trim(),
      areaCouncil: _council.wire,
      phone: _phone.text.trim(),
      feeKobo: nairaToKobo(_fee.text),
      collectEnabled: _collect,
    );
    final ok = await run(() async {
      final repo = ref.read(sellRepositoryProvider);
      final b = widget.branch;
      if (b == null) {
        await repo.addBranch(widget.store.id, input);
      } else {
        await repo.editBranch(widget.store.id, b.id, input);
      }
    });
    if (ok && mounted) Navigator.of(context).pop();
  }

  Future<void> _close() async {
    final b = widget.branch;
    if (b == null) return;
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text(SellStrings.closeBranch),
        content: const Text(SellStrings.closeBranchBody),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text(SellStrings.cancel),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text(SellStrings.closeBranch),
          ),
        ],
      ),
    );
    if (sure != true) return;
    final ok = await run(
      () => ref.read(sellRepositoryProvider).closeBranch(widget.store.id, b.id),
    );
    if (ok && mounted) Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    backgroundColor: context.tokens.bg,
    appBar: sellAppBar(
      context,
      widget.branch == null ? SellStrings.addBranch : SellStrings.branchesTitle,
    ),
    body: SafeArea(
      child: ListView(
        padding: const EdgeInsets.all(Tokens.s5),
        children: [
          AppTextField(
            label: SellStrings.branchName,
            controller: _name,
            maxLength: 60,
            capitalization: TextCapitalization.words,
            onChanged: (_) => setState(() {}),
          ),
          const SizedBox(height: Tokens.s3),
          AppTextField(
            label: SellStrings.pickupAddress,
            controller: _address,
            maxLines: 2,
            maxLength: 300,
            capitalization: TextCapitalization.words,
            onChanged: (_) => setState(() {}),
          ),
          const SizedBox(height: Tokens.s3),
          DropdownButtonFormField<AreaCouncil>(
            initialValue: _council,
            decoration: const InputDecoration(
              labelText: SellStrings.areaCouncil,
            ),
            items: [
              for (final c in AreaCouncil.values)
                DropdownMenuItem(value: c, child: Text(c.label)),
            ],
            onChanged: (c) => setState(() => _council = c ?? _council),
          ),
          const SizedBox(height: Tokens.s3),
          AppTextField(
            label: SellStrings.businessPhone,
            controller: _phone,
            keyboardType: TextInputType.phone,
            maxLength: 14,
            formatters: [FilteringTextInputFormatter.allow(RegExp(r'[0-9+]'))],
            onChanged: (_) => setState(() {}),
          ),
          const SizedBox(height: Tokens.s3),
          AppTextField(
            label: SellStrings.selfDeliveryFee,
            controller: _fee,
            keyboardType: const TextInputType.numberWithOptions(decimal: true),
            onChanged: (_) => setState(() {}),
          ),
          SwitchListTile(
            contentPadding: EdgeInsets.zero,
            title: const Text(SellStrings.collectSwitch),
            subtitle: _collect ? const Text(SellStrings.collectNotice) : null,
            value: _collect,
            onChanged: _toggleCollect,
          ),
          if (error != null) ...[
            const SizedBox(height: Tokens.s3),
            ErrorLine(error!),
          ],
          const SizedBox(height: Tokens.s4),
          LoadingButton(
            label: SellStrings.saveChanges,
            loading: busy,
            onPressed: _ready ? _save : null,
          ),
          if (widget.branch != null) ...[
            const SizedBox(height: Tokens.s2),
            TextButton(
              onPressed: busy ? null : _close,
              child: Text(
                SellStrings.closeBranch,
                style: TextStyle(color: context.tokens.errorText),
              ),
            ),
          ],
        ],
      ),
    ),
  );
}
