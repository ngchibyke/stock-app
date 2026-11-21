import 'package:market/features/addresses/address.dart';
import 'package:market/features/addresses/address_repository.dart';

/// A scriptable [AddressRepository] that records what it was asked.
class FakeAddressRepository implements AddressRepository {
  FakeAddressRepository([List<Address> initial = const []])
    : rows = [...initial];

  /// What the server "holds", newest first.
  final List<Address> rows;

  Future<List<Address>> Function()? onList;
  Future<Address> Function(NewAddress a)? onCreate;
  Future<void> Function(String id)? onRemove;

  int listCalls = 0;
  final List<NewAddress> created = [];
  final List<String> removed = [];

  @override
  Future<List<Address>> list() {
    listCalls++;
    final custom = onList;
    return custom != null ? custom() : Future.value([...rows]);
  }

  @override
  Future<Address> create(NewAddress a) {
    created.add(a);
    final custom = onCreate;
    if (custom != null) return custom(a);
    final made = Address(
      id: 'new-${created.length}',
      areaCouncil: a.areaCouncil,
      postcode: a.postcode,
      recipientName: a.recipientName,
      phone: a.phone,
      addressLine: a.addressLine,
      landmark: a.landmark,
    );
    rows.insert(0, made);
    return Future.value(made);
  }

  @override
  Future<void> remove(String id) {
    removed.add(id);
    final custom = onRemove;
    if (custom != null) return custom(id);
    rows.removeWhere((a) => a.id == id);
    return Future.value();
  }
}

Address testAddress({
  String id = 'a1',
  AreaCouncil? council = AreaCouncil.amac,
  String name = 'Ada Obi',
  String phone = '+2348031234567',
  String line = '12 Example Street, Wuse 2',
  String? landmark,
  String? postcode,
}) => Address(
  id: id,
  areaCouncil: council,
  postcode: postcode,
  recipientName: name,
  phone: phone,
  addressLine: line,
  landmark: landmark,
);
