## Summary

Describe the change and the problem it solves.

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor or performance
- [ ] Documentation or packaging
- [ ] RTL/bidi or shaping behaviour

## Verification

- [ ] `cargo test` passes
- [ ] `cargo check --all-targets` passes
- [ ] Installer still compiles when `installer/` changed
- [ ] Tested on Windows at 100% and scaled DPI when UI changed
- [ ] New or updated tests cover the change

## RTL and terminal safety

- [ ] Logical terminal text is unchanged; visual reordering stays in the paint path
- [ ] Selection, copy, cursor mapping and mouse coordinates still resolve correctly
- [ ] No telemetry, network calls or shell content in logs were added

## Screenshots

Add before/after images for visual changes.
