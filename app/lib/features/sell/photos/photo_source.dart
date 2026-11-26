import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:image_picker/image_picker.dart';

import '../sell_widgets.dart';
import 'camera_page.dart';

/// Where a product photo comes from: the guided camera or the gallery. Each answers
/// the picture's bytes, or null when the person backs out.
abstract class PhotoSource {
  Future<Uint8List?> fromCamera(BuildContext context, {required int shotIndex});
  Future<Uint8List?> fromGallery();
}

class DevicePhotoSource implements PhotoSource {
  const DevicePhotoSource();

  @override
  Future<Uint8List?> fromCamera(
    BuildContext context, {
    required int shotIndex,
  }) => pushPage<Uint8List>(context, GuidedCameraPage(shotIndex: shotIndex));

  @override
  Future<Uint8List?> fromGallery() async {
    final picked = await ImagePicker().pickImage(
      source: ImageSource.gallery,
      // The picture is cut and shrunk again before upload; this keeps the first
      // read light on a mid-range phone.
      maxWidth: 2400,
      maxHeight: 2400,
      requestFullMetadata: false,
    );
    return picked?.readAsBytes();
  }
}

final photoSourceProvider = Provider<PhotoSource>(
  (ref) => const DevicePhotoSource(),
);
