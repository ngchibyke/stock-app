import 'dart:typed_data';

import 'package:cached_network_image/cached_network_image.dart';
import 'package:material_ui/material_ui.dart';

import '../../../core/theme/tokens.dart';
import '../../../core/ui/pressable.dart';
import '../sell_strings.dart';

/// The most photos one product holds (the server takes 1 to 10).
const int maxProductPhotos = 10;

enum PhotoStatus { uploading, ready, failed }

/// One photo of a product being edited: already saved (a key and a link) or new
/// (bytes in memory while it uploads and until the page closes).
class PhotoEntry {
  PhotoEntry.saved({required String this.key, this.url})
    : bytes = null,
      status = PhotoStatus.ready;

  PhotoEntry.fresh(Uint8List this.bytes)
    : key = null,
      url = null,
      status = PhotoStatus.uploading;

  String? key;
  final String? url;
  final Uint8List? bytes;
  PhotoStatus status;
}

/// The photos of a product: a strip of squares (the first is the cover) and the
/// ways to add more.
class PhotoSection extends StatelessWidget {
  const PhotoSection({
    super.key,
    required this.entries,
    required this.onCamera,
    required this.onGallery,
    required this.onGuide,
    required this.onRemove,
    required this.onRetry,
    this.max = maxProductPhotos,
  });

  final List<PhotoEntry> entries;
  final VoidCallback onCamera;
  final VoidCallback onGallery;
  final VoidCallback onGuide;
  final void Function(int index) onRemove;
  final void Function(int index) onRetry;

  /// The most photos this screen takes (a product 10, a report or review 5).
  final int max;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    final full = entries.length >= max;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            Expanded(
              child: Text(
                SellStrings.photosTitle,
                style: theme.textTheme.titleMedium,
              ),
            ),
            TextButton(
              onPressed: onGuide,
              child: const Text(SellStrings.guideTitle),
            ),
          ],
        ),
        Text(
          SellStrings.photosHelp,
          style: theme.textTheme.bodyMedium?.copyWith(color: tokens.mute),
        ),
        const SizedBox(height: Tokens.s3),
        SizedBox(
          height: 96,
          child: ListView.separated(
            scrollDirection: Axis.horizontal,
            itemCount: entries.length,
            separatorBuilder: (_, _) => const SizedBox(width: Tokens.s2),
            itemBuilder: (context, i) => _Tile(
              entry: entries[i],
              onRemove: () => onRemove(i),
              onRetry: () => onRetry(i),
            ),
          ),
        ),
        const SizedBox(height: Tokens.s3),
        if (full)
          Text(SellStrings.maxPhotos, style: theme.textTheme.bodyMedium)
        else
          Row(
            children: [
              Expanded(
                child: OutlinedButton.icon(
                  onPressed: onCamera,
                  icon: const Icon(Icons.photo_camera_outlined),
                  label: const Text(SellStrings.takePhoto),
                ),
              ),
              const SizedBox(width: Tokens.s2),
              Expanded(
                child: OutlinedButton.icon(
                  onPressed: onGallery,
                  icon: const Icon(Icons.photo_library_outlined),
                  label: const Text(SellStrings.fromGallery),
                ),
              ),
            ],
          ),
      ],
    );
  }
}

class _Tile extends StatelessWidget {
  const _Tile({
    required this.entry,
    required this.onRemove,
    required this.onRetry,
  });

  final PhotoEntry entry;
  final VoidCallback onRemove;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final bytes = entry.bytes;
    final url = entry.url;
    final Widget picture = bytes != null
        ? Image.memory(
            bytes,
            fit: BoxFit.cover,
            cacheWidth: 200,
            gaplessPlayback: true,
          )
        : url != null
        ? CachedNetworkImage(
            imageUrl: url,
            fit: BoxFit.cover,
            memCacheWidth: 200,
          )
        : ColoredBox(color: tokens.line);
    return SizedBox(
      width: 96,
      height: 96,
      child: Stack(
        fit: StackFit.expand,
        children: [
          ClipRRect(
            borderRadius: BorderRadius.circular(Tokens.radius),
            child: picture,
          ),
          if (entry.status == PhotoStatus.uploading)
            Semantics(
              label: SellStrings.uploading,
              child: ColoredBox(
                color: tokens.bg.withValues(alpha: 0.6),
                child: const Center(
                  child: CircularProgressIndicator(strokeWidth: 3),
                ),
              ),
            ),
          if (entry.status == PhotoStatus.failed)
            Pressable(
              onTap: onRetry,
              semanticLabel: SellStrings.retryPhoto,
              child: ColoredBox(
                color: tokens.bg.withValues(alpha: 0.75),
                child: Center(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(Icons.refresh, color: tokens.errorText),
                      Text(
                        SellStrings.uploadFailed,
                        style: Theme.of(context).textTheme.labelSmall
                            ?.copyWith(color: tokens.errorText),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          Positioned(
            top: 0,
            right: 0,
            child: Pressable(
              onTap: onRemove,
              semanticLabel: SellStrings.removePhoto,
              child: SizedBox(
                width: Tokens.minTap - 8,
                height: Tokens.minTap - 8,
                child: Center(
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      color: tokens.card,
                      shape: BoxShape.circle,
                    ),
                    child: const Padding(
                      padding: EdgeInsets.all(2),
                      child: Icon(Icons.close, size: 16),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
