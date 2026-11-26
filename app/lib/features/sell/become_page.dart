import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/session/session_controller.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import 'sell_providers.dart';
import 'sell_repository.dart';
import 'sell_strings.dart';
import 'sell_widgets.dart';

/// Step one of becoming a seller: the access code the owner issued.
class BecomeSellerPage extends ConsumerStatefulWidget {
  const BecomeSellerPage({super.key, required this.onRedeemed});

  final VoidCallback onRedeemed;

  @override
  ConsumerState<BecomeSellerPage> createState() => _BecomeSellerPageState();
}

class _BecomeSellerPageState extends ConsumerState<BecomeSellerPage>
    with BusyAction {
  final TextEditingController _code = TextEditingController();

  @override
  void dispose() {
    _code.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    final code = _code.text.trim();
    if (code.isEmpty) return;
    final ok = await run(
      () => ref.read(sellRepositoryProvider).redeemCode(code),
    );
    if (!ok || !mounted) return;
    await ref.read(sessionProvider.notifier).refreshMe();
    ref.invalidate(myStoresProvider);
    if (mounted) widget.onRedeemed();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: sellAppBar(context, SellStrings.codeTitle),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.all(Tokens.s5),
          children: [
            Text(SellStrings.codeBody, style: theme.textTheme.bodyLarge),
            const SizedBox(height: Tokens.s4),
            AppTextField(
              label: SellStrings.codeLabel,
              controller: _code,
              capitalization: TextCapitalization.characters,
              action: TextInputAction.done,
              onChanged: (_) => setState(() {}),
              onSubmitted: (_) => _submit(),
            ),
            if (error != null) ...[
              const SizedBox(height: Tokens.s3),
              ErrorLine(error!),
            ],
            const SizedBox(height: Tokens.s4),
            LoadingButton(
              label: SellStrings.codeContinue,
              loading: busy,
              onPressed: _code.text.trim().isEmpty ? null : _submit,
            ),
          ],
        ),
      ),
    );
  }
}
