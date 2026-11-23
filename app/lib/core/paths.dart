/// Locations that pages push to without importing the router (which imports
/// the pages).
const String addressesPath = '/addresses';
const String newAddressPath = '/addresses/new';

/// The market.
const String homePath = '/home';

/// The list view of the market: search, categories, products and stores.
const String listPath = '/home/list';

/// A store's room (a deep link lands inside, with no flight).
String storeRoomPath(String slug) => '/home/store/${Uri.encodeComponent(slug)}';

String productPath(String id) => '/home/product/${Uri.encodeComponent(id)}';

/// A store as a plain page, from the list view.
String listStorePath(String slug) =>
    '/home/list/store/${Uri.encodeComponent(slug)}';

/// The pages under [homePath] that exist: anything else there goes home.
bool isMarketPath(String path) => _marketPaths.any((p) => p.hasMatch(path));

final List<RegExp> _marketPaths = [
  RegExp(r'^/home$'),
  RegExp(r'^/home/list$'),
  RegExp(r'^/home/store/[^/]+$'),
  RegExp(r'^/home/product/[^/]+$'),
  RegExp(r'^/home/list/store/[^/]+$'),
];
const String cartPath = '/cart';
