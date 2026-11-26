import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/format/phone.dart';
import '../../core/paths.dart';
import '../../core/session/session_controller.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import '../auth/auth_errors.dart';
import 'address.dart';
import 'address_errors.dart';
import 'address_repository.dart';

/// Which form field a server `validation_failed` message is about, through a
/// mapping kept here: the server's own words are never shown.
enum _Field { name, phone, line, landmark, postcode, council }

_Field? _fieldNamedBy(String message) {
  final m = message.toLowerCase();
  if (m.contains('landmark')) return _Field.landmark;
  if (m.contains('recipient') || m.contains('name')) return _Field.name;
  if (m.contains('phone')) return _Field.phone;
  if (m.contains('postcode')) return _Field.postcode;
  if (m.contains('council') || m.contains('area')) return _Field.council;
  if (m.contains('address')) return _Field.line;
  return null;
}

/// Add a delivery address. The recipient and phone start as the person's own;
/// nothing typed here is stored anywhere but the server, and it is never
/// logged.
class AddressFormPage extends ConsumerStatefulWidget {
  const AddressFormPage({super.key});

  @override
  ConsumerState<AddressFormPage> createState() => _AddressFormPageState();
}

class _AddressFormPageState extends ConsumerState<AddressFormPage> {
  late final TextEditingController _name;
  late final TextEditingController _phone;
  final _line = TextEditingController();
  final _landmark = TextEditingController();
  final _postcode = TextEditingController();

  AreaCouncil? _council;

  String? _nameError;
  String? _phoneError;
  String? _lineError;
  String? _landmarkError;
  String? _postcodeError;
  String? _councilError;
  String? _formError;
  bool _retryable = false;
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    final state = ref.read(sessionProvider).value;
    final me = state is SessionSignedIn ? state.me : null;
    _name = TextEditingController(text: me?.displayName ?? '');
    _phone = TextEditingController(
      text: me == null ? '' : displayPhone(me.phone),
    );
  }

  @override
  void dispose() {
    _name.dispose();
    _phone.dispose();
    _line.dispose();
    _landmark.dispose();
    _postcode.dispose();
    super.dispose();
  }

  bool _validate() {
    final name = _name.text.trim();
    final phone = _phone.text.trim();
    final line = _line.text.trim();
    final landmark = _landmark.text.trim();

    final nameError = name.isEmpty
        ? Strings.recipientRequired
        : name.length > AddressLimits.recipientName
        ? Strings.recipientTooLong
        : null;
    final phoneError = looksLikeNigerianPhone(phone)
        ? null
        : Strings.phoneInvalid;
    final lineError = line.isEmpty
        ? Strings.addressLineRequired
        : line.length > AddressLimits.addressLine
        ? Strings.addressLineTooLong
        : null;
    final landmarkError = landmark.length > AddressLimits.landmark
        ? Strings.landmarkTooLong
        : null;
    final councilError = _council == null ? Strings.councilRequired : null;

    setState(() {
      _nameError = nameError;
      _phoneError = phoneError;
      _lineError = lineError;
      _landmarkError = landmarkError;
      _postcodeError = null;
      _councilError = councilError;
      _formError = null;
    });
    return nameError == null &&
        phoneError == null &&
        lineError == null &&
        landmarkError == null &&
        councilError == null;
  }

  Future<void> _submit() async {
    if (_busy) return;
    if (!_validate()) return;
    final address = NewAddress(
      recipientName: _name.text.trim(),
      phone: _phone.text.trim(),
      addressLine: _line.text.trim(),
      landmark: _landmark.text.trim(),
      areaCouncil: _council!,
      postcode: _postcode.text.trim(),
    );
    setState(() {
      _busy = true;
      _formError = null;
    });
    try {
      await ref.read(addressesProvider.notifier).create(address);
      if (!mounted) return;
      setState(() => _busy = false);
      if (context.canPop()) {
        context.pop();
      } else {
        context.go(addressesPath);
      }
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _retryable = authErrorIsRetryable(e);
        _showServerError(e);
      });
    }
  }

  void _showServerError(Object error) {
    if (error is ApiError) {
      switch (error.code) {
        case 'invalid_phone':
          _phoneError = Strings.phoneInvalid;
          return;
        case 'invalid_postcode':
          _postcodeError = Strings.postcodeInvalid;
          return;
        case 'outside_service_area':
          _formError = Strings.outsideServiceArea;
          return;
        case 'too_many_addresses':
          _formError = Strings.tooManyAddresses;
          return;
        case 'validation_failed':
          switch (_fieldNamedBy(error.message)) {
            case _Field.name:
              _nameError = Strings.recipientInvalid;
            case _Field.phone:
              _phoneError = Strings.phoneInvalid;
            case _Field.line:
              _lineError = Strings.addressLineInvalid;
            case _Field.landmark:
              _landmarkError = Strings.landmarkInvalid;
            case _Field.postcode:
              _postcodeError = Strings.postcodeInvalid;
            case _Field.council:
              _councilError = Strings.councilRequired;
            case null:
              _formError = Strings.checkDetails;
          }
          return;
      }
    }
    _formError = addressErrorMessage(error);
  }

  /// A field's error goes as soon as the person edits that field.
  void _clearError(_Field field) {
    setState(() {
      switch (field) {
        case _Field.name:
          _nameError = null;
        case _Field.phone:
          _phoneError = null;
        case _Field.line:
          _lineError = null;
        case _Field.landmark:
          _landmarkError = null;
        case _Field.postcode:
          _postcodeError = null;
        case _Field.council:
          _councilError = null;
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final error = _formError;
    return Scaffold(
      appBar: AppBar(
        title: Semantics(
          header: true,
          child: const Text(Strings.newAddressTitle),
        ),
      ),
      // The Scaffold lifts the body above the keyboard and the focused field
      // scrolls into view inside it.
      body: SafeArea(
        child: SingleChildScrollView(
          padding: const EdgeInsets.all(Tokens.s5),
          keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              AppTextField(
                label: Strings.recipientNameLabel,
                controller: _name,
                errorText: _nameError,
                keyboardType: TextInputType.name,
                capitalization: TextCapitalization.words,
                action: TextInputAction.next,
                maxLength: AddressLimits.recipientName,
                maxLines: 2,
                onChanged: (_) => _clearError(_Field.name),
              ),
              const SizedBox(height: Tokens.s4),
              AppTextField(
                label: Strings.phoneLabel,
                controller: _phone,
                errorText: _phoneError,
                keyboardType: TextInputType.phone,
                formatters: const [PhoneInputFormatter()],
                action: TextInputAction.next,
                onChanged: (_) => _clearError(_Field.phone),
              ),
              const SizedBox(height: Tokens.s4),
              AppTextField(
                label: Strings.addressLineLabel,
                controller: _line,
                errorText: _lineError,
                keyboardType: TextInputType.streetAddress,
                capitalization: TextCapitalization.sentences,
                action: TextInputAction.next,
                maxLength: AddressLimits.addressLine,
                maxLines: 4,
                onChanged: (_) => _clearError(_Field.line),
              ),
              const SizedBox(height: Tokens.s4),
              AppTextField(
                label: Strings.landmarkLabel,
                controller: _landmark,
                errorText: _landmarkError,
                capitalization: TextCapitalization.sentences,
                action: TextInputAction.next,
                maxLength: AddressLimits.landmark,
                maxLines: 3,
                onChanged: (_) => _clearError(_Field.landmark),
              ),
              const SizedBox(height: Tokens.s4),
              DropdownButtonFormField<AreaCouncil>(
                initialValue: _council,
                isExpanded: true,
                decoration: InputDecoration(
                  labelText: Strings.councilLabel,
                  errorText: _councilError,
                ),
                items: [
                  for (final c in AreaCouncil.values)
                    DropdownMenuItem(value: c, child: Text(c.label)),
                ],
                onChanged: (c) {
                  setState(() => _council = c);
                  _clearError(_Field.council);
                },
              ),
              const SizedBox(height: Tokens.s4),
              AppTextField(
                label: Strings.postcodeLabel,
                controller: _postcode,
                errorText: _postcodeError,
                helperText: Strings.postcodeHint,
                capitalization: TextCapitalization.characters,
                action: TextInputAction.done,
                maxLength: AddressLimits.postcode,
                onChanged: (_) => _clearError(_Field.postcode),
                onSubmitted: (_) => _submit(),
              ),
              if (error != null) ...[
                const SizedBox(height: Tokens.s3),
                AuthErrorBanner(
                  message: error,
                  onRetry: _retryable && !_busy ? _submit : null,
                ),
              ],
              const SizedBox(height: Tokens.s5),
              LoadingButton(
                label: Strings.saveAddress,
                loading: _busy,
                onPressed: _submit,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
