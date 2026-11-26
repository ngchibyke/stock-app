/// The rules of the payment view, kept pure so they are tested without a browser.
library;

/// Whether [url] is the page the provider sends the buyer back to: the same scheme,
/// host, port and path as [returnUrl] (the query, which the provider may add to,
/// is ignored). With no configured return page nothing matches.
bool isReturnUrl(String url, String? returnUrl) {
  if (returnUrl == null) return false;
  final a = Uri.tryParse(url);
  final b = Uri.tryParse(returnUrl);
  if (a == null || b == null) return false;
  return a.scheme == b.scheme &&
      a.host == b.host &&
      a.port == b.port &&
      _path(a) == _path(b);
}

String _path(Uri u) => u.path.endsWith('/') && u.path.length > 1
    ? u.path.substring(0, u.path.length - 1)
    : u.path;

/// Whether the payment view may open [url]: https pages (the provider and the
/// bank's own security pages move between hosts), and the return page, which
/// may be plain http only on a loopback host in development. Everything else
/// (other schemes, app links, files) is refused.
bool mayNavigate(String url, String? returnUrl) {
  final u = Uri.tryParse(url);
  if (u == null || u.host.isEmpty) return false;
  if (isReturnUrl(url, returnUrl)) return true;
  return u.scheme == 'https';
}

/// Whether the address the server sent for the payment may be opened at all.
bool mayStartPayment(String url) {
  final u = Uri.tryParse(url);
  if (u == null || u.host.isEmpty) return false;
  if (u.scheme == 'https') return true;
  return u.scheme == 'http' &&
      const {'localhost', '127.0.0.1', '10.0.2.2'}.contains(u.host);
}
