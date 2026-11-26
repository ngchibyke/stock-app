// Applies brand.json to the Android and iOS projects, or with --check says which files
// differ. Run from app/:  dart run tool/apply_brand.dart [--check]
import 'dart:io';

import 'brand.dart';

void main(List<String> args) {
  final check = args.contains('--check');
  final Brand brand;
  try {
    brand = Brand.read('.');
  } on FormatException catch (e) {
    stderr.writeln('brand.json: ${e.message}');
    exit(2);
  }
  if (check) {
    final bad = drifted('.', brand);
    if (bad.isEmpty) {
      stdout.writeln('brand.json matches every platform file');
      return;
    }
    stdout.writeln('these files differ from brand.json:');
    for (final f in bad) {
      stdout.writeln('  $f');
    }
    stdout.writeln('run: dart run tool/apply_brand.dart');
    exit(1);
  }
  final changed = apply('.', brand);
  stdout.writeln(
    changed.isEmpty
        ? 'nothing to change'
        : 'updated ${changed.length} files:\n${changed.map((f) => '  $f').join('\n')}',
  );
}
