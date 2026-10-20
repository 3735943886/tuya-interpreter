# Golden reference tooling

These scripts extract reference data for interpreter regression tests from Home Assistant's
Tuya integration and `tuya-device-handlers`. Fixtures and generated snapshots retain their
upstream attribution; see [third-party notices](../../THIRD_PARTY_NOTICES.md).

## Data and consumers

| Tool or data | Purpose |
| --- | --- |
| `ambr.py`, `build_golden.py` | Parse Home Assistant snapshot files into classification and value reference data. |
| `quirk_oracle.py` | Record post-quirk device schemas using real fixtures. |
| `quirk_oracle_synthetic.py` | Record all registered quirks against empty-schema devices. |
| `extract_actions.py` | Extract service-call and command cases from reference tests. |
| `tests/golden/fixtures.py` | Load fixtures with reference-compatible JSON normalization. |
| `tests/test_golden.py` | Compare classification and values against golden data. |
| `tests/test_quirks.py` | Check real-fixture and synthetic quirk patches. |

Oracle scripts require `tuya-device-handlers==0.0.29` and
`tuya-device-sharing-sdk==0.2.15`. Review each script's source paths before running it
against an upstream checkout. Regeneration writes reference JSON beside these scripts;
review generated changes before committing.

Home Assistant baseline snapshots use `no_quirk`, so the baseline classification data has
quirks disabled. Quirk behavior is checked separately. Fixture loading preserves the
reference's compact JSON serialization, reserializes JSON status values, and replaces
redacted status strings with empty strings.

## Coverage limits

Synthetic empty-schema quirk cases verify declared patches. They do not exercise every
condition that depends on an existing schema or initial status; dedicated regression tests
cover additional cases. Some snapshot fields, including enabled-by-default flags for
button, binary-sensor and sensor entities, are not independently checked by the snapshots.
Service calls and update behavior require action and driver tests in addition to snapshots.

Use `scripts/gen_core_tables.py` to regenerate category tables and
`scripts/build_rules.py --tables` to fold them into `rules/builtin/00-default.json`.
Expected data changes require review against intended behavior and the upstream reference.
