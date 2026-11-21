import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/api/providers.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/features/addresses/address.dart';
import 'package:market/features/addresses/address_repository.dart';

import '../../support/app_harness.dart';
import '../../support/fakes.dart';

AddressRepository _repo(FakeAdapter adapter) => ApiAddressRepository(
  ApiClient(
    baseUrl: 'https://api.test',
    readToken: () async => 'tok',
    onUnauthorized: () {},
    adapter: adapter,
  ),
);

Map<String, Object?> _row({
  String id = 'a1',
  String council = 'amac',
  String? postcode = 'FC123456789',
  String? courier = 'C-1',
  String? landmark = 'Opposite the mosque',
}) => {
  'id': id,
  'area_council': council,
  'postcode': postcode,
  'courier_address_code': courier,
  'recipient_name': 'Ada Obi',
  'phone': '+2348031234567',
  'address_line': '12 Example Street, Wuse 2',
  'landmark': landmark,
};

const _new = NewAddress(
  recipientName: 'Ada Obi',
  phone: '0803 123 4567',
  addressLine: '12 Example Street, Wuse 2',
  landmark: 'Opposite the mosque',
  areaCouncil: AreaCouncil.gwagwalada,
  postcode: 'FC123456789',
);

void main() {
  test('list_parses_every_field_including_nulls', () async {
    final adapter = FakeAdapter()
      ..reply(200, [
        _row(),
        _row(id: 'a2', postcode: null, courier: null, landmark: null),
      ]);

    final list = await _repo(adapter).list();

    expect(adapter.requests.single.method, 'GET');
    expect(adapter.requests.single.path, '/me/addresses');
    expect(list.map((a) => a.id), ['a1', 'a2']);
    final first = list.first;
    expect(first.areaCouncil, AreaCouncil.amac);
    expect(first.postcode, 'FC123456789');
    expect(first.recipientName, 'Ada Obi');
    expect(first.phone, '+2348031234567');
    expect(first.addressLine, '12 Example Street, Wuse 2');
    expect(first.landmark, 'Opposite the mosque');
    final second = list.last;
    expect(second.postcode, isNull);
    expect(second.landmark, isNull);
  });

  test('every_council_parses_and_an_unknown_one_never_crashes', () {
    for (final c in AreaCouncil.values) {
      expect(AreaCouncil.fromWire(c.wire), c);
    }
    expect(AreaCouncil.amac.label, 'AMAC (Abuja Municipal)');
    expect(AreaCouncil.bwari.label, 'Bwari');
    expect(AreaCouncil.gwagwalada.label, 'Gwagwalada');
    expect(AreaCouncil.kuje.label, 'Kuje');
    expect(AreaCouncil.kwali.label, 'Kwali');
    expect(AreaCouncil.abaji.label, 'Abaji');
    expect(AreaCouncil.fromWire('lagos-island'), isNull);
    expect(AreaCouncil.fromWire(null), isNull);
    expect(AreaCouncil.fromWire(7), isNull);

    final unknown = Address.fromJson(_row(council: 'lagos-island'));
    expect(unknown.areaCouncil, isNull);
    expect(unknown.councilLabel, isNot(contains('lagos')));
  });

  test('odd_json_never_throws_and_a_bad_row_does_not_blank_the_list', () async {
    final adapter = FakeAdapter()
      ..reply(200, [
        'not a row',
        42,
        null,
        {'recipient_name': 'No id'},
        {'id': 5, 'recipient_name': 'Numeric id'},
        {
          'id': 'odd',
          'area_council': 12,
          'postcode': 9,
          'recipient_name': null,
          'phone': [],
          'address_line': {},
          'landmark': true,
        },
        _row(id: 'good'),
      ]);

    final list = await _repo(adapter).list();

    // Rows with no usable id cannot be shown or deleted, so they are skipped;
    // a row with odd fields is kept with plain fallbacks.
    expect(list.map((a) => a.id), ['odd', 'good']);
    final odd = list.first;
    expect(odd.areaCouncil, isNull);
    expect(odd.postcode, isNull);
    expect(odd.landmark, isNull);
    expect(odd.recipientName, '');
    expect(odd.phone, '');
    expect(odd.addressLine, '');
  });

  test('a_list_that_is_not_a_list_is_a_bad_response', () async {
    final adapter = FakeAdapter()..reply(200, {'oops': true});
    await expectLater(
      _repo(adapter).list(),
      throwsA(isA<ApiError>().having((e) => e.code, 'code', 'bad_response')),
    );
  });

  test('create_posts_the_exact_body_with_the_wire_council_name', () async {
    final adapter = FakeAdapter()
      ..reply(201, _row(id: 'new', council: 'gwagwalada'));

    final created = await _repo(adapter).create(_new);

    final sent = adapter.requests.single;
    expect(sent.method, 'POST');
    expect(sent.path, '/me/addresses');
    expect(sent.body, {
      'recipient_name': 'Ada Obi',
      'phone': '0803 123 4567',
      'address_line': '12 Example Street, Wuse 2',
      'landmark': 'Opposite the mosque',
      'area_council': 'gwagwalada',
      'postcode': 'FC123456789',
    });
    expect(created.id, 'new');
    expect(created.areaCouncil, AreaCouncil.gwagwalada);
  });

  test('a_blank_landmark_and_postcode_are_left_out_of_the_body', () {
    for (final blank in [null, '', '   ']) {
      final json = NewAddress(
        recipientName: 'Ada',
        phone: '08031234567',
        addressLine: '1 Road',
        landmark: blank,
        areaCouncil: AreaCouncil.kuje,
        postcode: blank,
      ).toJson();
      expect(json.containsKey('landmark'), isFalse, reason: '$blank');
      expect(json.containsKey('postcode'), isFalse, reason: '$blank');
      expect(json['area_council'], 'kuje');
    }
  });

  test('remove_calls_delete_and_the_list_drops_the_row', () async {
    final h = AppHarness(store: MemorySessionStore('tok'));
    h.adapter
      ..reply(200, testMe)
      ..reply(200, [_row(), _row(id: 'a2')])
      ..reply(204);
    final container = ProviderContainer(overrides: h.pageOverrides);
    addTearDown(container.dispose);
    await container.read(sessionProvider.future);
    final sub = container.listen(addressesProvider, (_, _) {});
    addTearDown(sub.close);

    expect((await container.read(addressesProvider.future)).length, 2);
    await container.read(addressesProvider.notifier).remove('a1');

    final delete = h.adapter.requests.last;
    expect(delete.method, 'DELETE');
    expect(delete.path, '/me/addresses/a1');
    expect(container.read(addressesProvider).value!.map((a) => a.id), ['a2']);
  });

  test('a_failed_remove_keeps_the_row', () async {
    final h = AppHarness(store: MemorySessionStore('tok'));
    h.adapter
      ..reply(200, testMe)
      ..reply(200, [_row()])
      ..reply(500, {
        'error': {'code': 'internal', 'message': 'boom'},
      });
    final container = ProviderContainer(overrides: h.pageOverrides);
    addTearDown(container.dispose);
    await container.read(sessionProvider.future);
    final sub = container.listen(addressesProvider, (_, _) {});
    addTearDown(sub.close);
    await container.read(addressesProvider.future);

    await expectLater(
      container.read(addressesProvider.notifier).remove('a1'),
      throwsA(isA<ApiError>()),
    );
    expect(container.read(addressesProvider).value!.map((a) => a.id), ['a1']);
  });

  test('a_created_address_goes_to_the_top_of_the_held_list', () async {
    final h = AppHarness(store: MemorySessionStore('tok'));
    h.adapter
      ..reply(200, testMe)
      ..reply(200, [_row(id: 'old')])
      ..reply(201, _row(id: 'fresh'));
    final container = ProviderContainer(overrides: h.pageOverrides);
    addTearDown(container.dispose);
    await container.read(sessionProvider.future);
    final sub = container.listen(addressesProvider, (_, _) {});
    addTearDown(sub.close);
    await container.read(addressesProvider.future);

    await container.read(addressesProvider.notifier).create(_new);

    expect(container.read(addressesProvider).value!.map((a) => a.id), [
      'fresh',
      'old',
    ]);
    // One list call and one create: no reload.
    expect(
      h.adapter.requests.where((r) => r.path == '/me/addresses').length,
      2,
    );
  });

  test('the_held_list_is_dropped_when_the_person_signs_out', () async {
    final h = AppHarness(store: MemorySessionStore('tok'));
    h.adapter
      ..reply(200, testMe)
      ..reply(200, [_row()])
      ..reply(200, <String, dynamic>{});
    final container = ProviderContainer(overrides: h.pageOverrides);
    addTearDown(container.dispose);
    await container.read(sessionProvider.future);
    final sub = container.listen(addressesProvider, (_, _) {});
    addTearDown(sub.close);
    expect((await container.read(addressesProvider.future)).length, 1);

    await container.read(sessionProvider.notifier).signOut();
    final after = await container.read(addressesProvider.future);

    expect(after, isEmpty);
    // Signed out: no further call asked for addresses.
    expect(
      h.adapter.requests.where((r) => r.path == '/me/addresses').length,
      1,
    );
  });

  test('server_errors_map_to_api_errors', () async {
    for (final code in [
      'outside_service_area',
      'invalid_postcode',
      'too_many_addresses',
      'validation_failed',
      'invalid_phone',
    ]) {
      final adapter = FakeAdapter()
        ..reply(422, {
          'error': {'code': code, 'message': 'server words for $code'},
        });
      await expectLater(
        _repo(adapter).create(_new),
        throwsA(
          isA<ApiError>()
              .having((e) => e.code, 'code', code)
              .having((e) => e.status, 'status', 422),
        ),
        reason: code,
      );
    }
  });

  test('the_api_client_provider_is_used_for_the_repository', () {
    final h = AppHarness();
    final container = ProviderContainer(overrides: h.pageOverrides);
    addTearDown(container.dispose);
    expect(
      container.read(addressRepositoryProvider),
      isA<ApiAddressRepository>(),
    );
    expect(container.read(apiClientProvider), isA<ApiClient>());
  });
}
