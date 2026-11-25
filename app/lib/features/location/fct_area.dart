/// A box around the Federal Capital Territory. It is a slight over-estimate
/// near the borders (Suleja falls inside), which is fine for a hint: the
/// delivery-address rule on the server decides what can be delivered.
const double _minLat = 8.40;
const double _maxLat = 9.35;
const double _minLng = 6.75;
const double _maxLng = 7.62;

/// Whether a position is (roughly) in the FCT.
bool inFct(double lat, double lng) =>
    lat >= _minLat && lat <= _maxLat && lng >= _minLng && lng <= _maxLng;
