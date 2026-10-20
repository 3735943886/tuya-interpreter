# Publishing tuya-interpreter

A pushed `interpreter-v<SemVer>` tag publishes the Rust crate and Python distributions
under the name `tuya-interpreter`. For the first stable release:

| Item | Version |
| --- | --- |
| Git tag | `interpreter-v0.1.0` |
| crates.io | `0.1.0` |
| PyPI | `0.1.0` |

Cargo uses SemVer; Python uses PEP 440. Prereleases use `-alpha.N`, `-beta.N` or `-rc.N`
in Cargo and the tag, mapped to `aN`, `bN` or `rcN` in Python.
`scripts/release_version.py` validates the exact tag against Cargo and supplies the
Python version to every release build.

## Registry configuration

Both registries use the repository `3735943886/tuya-interpreter` and workflow `release.yml`.

| Registry | GitHub environment | Authentication |
| --- | --- | --- |
| PyPI | `pypi` | Trusted Publishing through OIDC |
| crates.io | `crates-io` | Trusted Publishing through OIDC, or `CARGO_REGISTRY_TOKEN` |

Configure the matching publisher in each registry. A configured Cargo token takes
precedence over OIDC; authentication failures do not fall back to another method.

## Preparing a release

1. Set `rust/engine/Cargo.toml` to the intended SemVer version and update installation examples.
2. Check dependency resolution with `cargo check --manifest-path rust/Cargo.toml` and
   `cargo check --manifest-path examples/rust/Cargo.toml`. Cargo lockfiles are ignored;
   fresh checkouts resolve the versions allowed by each manifest.
3. Review the IL specification pinned in `tests/ildevice.ref` and follow the
   [versioning policy](../README.md#versioning-and-the-il-specification).
4. Review public documentation and package contents. Retain third-party notices and
   exclude local credentials, agent notes, caches and build outputs.
5. Commit the changes. Verify CI and run the Release workflow manually on that commit
   to validate all artifacts without publishing.
6. Push the matching tag. For `0.1.0`:

```sh
python scripts/release_version.py --tag interpreter-v0.1.0
git tag interpreter-v0.1.0
git push origin interpreter-v0.1.0
```

Only pushed version tags publish. Pull requests and manual runs build and test without
publishing. A malformed tag or a tag that differs from Cargo fails before publication.
Existing tags must not be deleted, renamed or overwritten.

Both publisher jobs wait for the full test job, all 13 Python wheel builds, the source
archive and verified crate packaging. The workflow stores Python distributions and the
`.crate` file as artifacts. The registries publish independently; if one succeeds and the
other fails, rerun only the failed publisher job. Published versions cannot be overwritten.

## Installing the stable release

```sh
pip install 'tuya-interpreter==0.1.0'
```

```toml
[dependencies]
tuya-interpreter = "=0.1.0"
```

Python imports and Rust crate references use `tuya_interpreter`.
