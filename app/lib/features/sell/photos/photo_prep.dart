import 'dart:isolate';
import 'dart:typed_data';

import 'package:image/image.dart' as img;

/// The longest edge of an uploaded photo, in pixels.
const int photoMaxEdge = 1600;

/// JPEG quality of an uploaded photo (a typical photo lands near 300 KB).
const int photoQuality = 82;

/// The side of the centred square cut from a [width] by [height] picture.
int squareSide(int width, int height) => width < height ? width : height;

/// The edge a [side]-pixel square is resized to: never enlarged.
int targetEdge(int side, {int maxEdge = photoMaxEdge}) =>
    side > maxEdge ? maxEdge : side;

/// Makes a camera or gallery picture ready to upload: turned upright by its
/// orientation, cut to the centred square, shrunk to [photoMaxEdge] and saved
/// again as JPEG with every tag cleared, so a location in the original never
/// leaves the phone. Returns null when [bytes] is not a picture.
///
/// Heavy: call [preparePhoto] so it runs off the main thread.
Uint8List? preparePhotoSync(Uint8List bytes) {
  final decoded = img.decodeImage(bytes);
  if (decoded == null) return null;
  final upright = img.bakeOrientation(decoded);
  final side = squareSide(upright.width, upright.height);
  final cut = img.copyCrop(
    upright,
    x: (upright.width - side) ~/ 2,
    y: (upright.height - side) ~/ 2,
    width: side,
    height: side,
  );
  final edge = targetEdge(side);
  final sized = edge == side
      ? cut
      : img.copyResize(
          cut,
          width: edge,
          height: edge,
          interpolation: img.Interpolation.average,
        );
  // Cropping and resizing keep the original's tags, so they are cleared here: a
  // location in the original must not go with the upload.
  sized.exif = img.ExifData();
  return Uint8List.fromList(img.encodeJpg(sized, quality: photoQuality));
}

/// [preparePhotoSync] in an isolate, so the screen keeps moving.
Future<Uint8List?> preparePhoto(Uint8List bytes) =>
    Isolate.run(() => preparePhotoSync(bytes));
