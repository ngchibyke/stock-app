import 'package:json_annotation/json_annotation.dart';

part 'me.g.dart';

/// The signed-in user, as answered by `GET /me`.
@JsonSerializable(createToJson: false)
class Me {
  const Me({
    required this.id,
    required this.displayName,
    required this.phone,
    required this.role,
    required this.isSeller,
  });

  factory Me.fromJson(Map<String, dynamic> json) => _$MeFromJson(json);

  final String id;
  @JsonKey(name: 'display_name')
  final String displayName;
  final String phone;
  final String role;
  @JsonKey(name: 'is_seller')
  final bool isSeller;
}
