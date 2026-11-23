import 'package:material_ui/material_ui.dart' hide Page;

import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import '../catalog/catalog_models.dart';

/// A list loaded a page at a time by cursor, the next page asked for as the
/// list nears its end (once per cursor). Give it a new [Key] to start over.
class PagedList<T> extends StatefulWidget {
  const PagedList({
    super.key,
    required this.load,
    required this.itemBuilder,
    required this.empty,
    this.header,
  });

  final Future<Page<T>> Function(String? cursor) load;
  final Widget Function(BuildContext context, T item) itemBuilder;

  /// Shown when the first page comes back with nothing.
  final Widget empty;
  final Widget? header;

  @override
  State<PagedList<T>> createState() => _PagedListState<T>();
}

class _PagedListState<T> extends State<PagedList<T>> {
  final List<T> _items = [];
  String? _cursor;
  bool _started = false;
  bool _loading = false;
  Object? _error;

  /// Cursors already asked for, so one is never fetched twice.
  final Set<String?> _asked = {};

  bool get _more => _cursor != null;

  @override
  void initState() {
    super.initState();
    _next();
  }

  Future<void> _next() async {
    if (_loading || (_started && !_more)) return;
    final cursor = _cursor;
    if (!_asked.add(cursor)) return;
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final page = await widget.load(cursor);
      if (!mounted) return;
      setState(() {
        _items.addAll(page.items);
        _cursor = page.nextCursor;
        _started = true;
      });
    } catch (e) {
      _asked.remove(cursor);
      if (mounted) setState(() => _error = e);
    } finally {
      if (mounted) setState(() => _loading = false);
    }
  }

  bool _onScroll(ScrollNotification n) {
    if (n.metrics.extentAfter < 400) _next();
    return false;
  }

  @override
  Widget build(BuildContext context) {
    if (!_started) {
      if (_error != null) {
        return ErrorState(message: Strings.listLoadFailed, onRetry: _next);
      }
      return ListView(
        padding: const EdgeInsets.all(Tokens.s4),
        children: [
          ?widget.header,
          for (var i = 0; i < 6; i++)
            const Padding(
              padding: EdgeInsets.only(bottom: Tokens.s3),
              child: Skeleton(height: 64),
            ),
        ],
      );
    }
    if (_items.isEmpty) {
      return ListView(
        padding: const EdgeInsets.all(Tokens.s4),
        children: [
          ?widget.header,
          const SizedBox(height: Tokens.s5),
          widget.empty,
        ],
      );
    }
    final header = widget.header;
    final offset = header == null ? 0 : 1;
    return NotificationListener<ScrollNotification>(
      onNotification: _onScroll,
      child: ListView.separated(
        padding: const EdgeInsets.fromLTRB(
          Tokens.s4,
          Tokens.s2,
          Tokens.s4,
          Tokens.s5,
        ),
        itemCount: offset + _items.length + (_more || _error != null ? 1 : 0),
        separatorBuilder: (_, i) => i < offset
            ? const SizedBox.shrink()
            : const SizedBox(height: Tokens.s2),
        itemBuilder: (context, i) {
          if (header != null && i == 0) return header;
          final at = i - offset;
          if (at < _items.length) {
            return widget.itemBuilder(context, _items[at]);
          }
          if (_error != null) {
            return Center(
              child: TextButton(
                onPressed: _next,
                child: const Text(Strings.retry),
              ),
            );
          }
          return const Skeleton(height: 64);
        },
      ),
    );
  }
}
