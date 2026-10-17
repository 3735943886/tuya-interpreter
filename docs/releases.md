# Publishing tuya-interpreter

One tag publishes the Rust crate and Python distributions under the same name,
`tuya-interpreter`. The first published release is:

| Item | Version |
| --- | --- |
| Git tag | `v0.1.0-alpha.1` |
| crates.io | `0.1.0-alpha.1` |
| PyPI | `0.1.0a1` |

Cargo uses SemVer prereleases; Python uses PEP 440. `scripts/release_version.py`
validates the Cargo version and exact tag, then supplies the corresponding Python version
explicitly to all release builds. This avoids inheriting the old project's `0.4.*` tags.
Alpha, beta and release-candidate tags use `-alpha.N`, `-beta.N` and `-rc.N` respectively.
Future tags use `interpreter-v<SemVer>`, such as `interpreter-v0.1.0-alpha.2`
or `interpreter-v0.4.0`. Stable tags use `interpreter-vX.Y.Z`. Existing `v*` tags are
preserved and do not trigger new releases. SCM selects the new namespace, with exactly
`v0.1.0-alpha.1` retained as the already published baseline.

## One-time registry configuration

### PyPI

Create a pending Trusted Publisher for the new project if it has not been registered:

- Project: `tuya-interpreter`
- Owner: `3735943886`
- Repository: `tuya-interpreter`
- Workflow: `release.yml`
- GitHub environment: `pypi`

The workflow uses OIDC through `pypa/gh-action-pypi-publish`; it does not require a PyPI API token.
Configure the matching publisher at [PyPI Publishing](https://pypi.org/manage/account/publishing/).

### crates.io

The first registration requires an API token because Trusted Publishing currently requires
an already published crate. See [crates.io Trusted Publishing](https://crates.io/docs/trusted-publishing).

Create a crates.io API token with permission to publish the new crate, and store it as
`CARGO_REGISTRY_TOKEN` in the GitHub `crates-io` environment or repository secrets.
Create the `crates-io` GitHub environment as needed. The first tagged run uses this token.

After the initial release, configure the crate's Trusted Publisher with:

- Owner: `3735943886`
- Repository: `tuya-interpreter`
- Workflow: `release.yml`
- GitHub environment: `crates-io`

Then remove the GitHub token secret to use `rust-lang/crates-io-auth-action` and short-lived
OIDC credentials. A configured token takes precedence; there is no fallback after an OIDC failure.

## Preparing a release

1. Set `rust/engine/Cargo.toml` to the intended SemVer version.
2. Check dependency resolution with `cargo check --manifest-path rust/Cargo.toml` and
   `cargo check --manifest-path examples/rust/Cargo.toml`. Cargo lockfiles are ignored;
   fresh checkouts resolve the latest versions allowed by each manifest.
3. Commit the changes and run the Release workflow manually to validate and build artifacts.
4. Push the matching version tag when ready to publish. For the next alpha, first set
   Cargo to `0.1.0-alpha.2`, then:

```sh
git tag interpreter-v0.1.0-alpha.2
git push origin interpreter-v0.1.0-alpha.2
```

PR and manual runs build the candidate version from Cargo without publishing.
Even a manual run on a tag does not publish. Only a pushed version tag triggers the publisher jobs.
Malformed tags or a tag that differs from Cargo fail before build/publish.

Both publisher jobs wait for the full test job, all 13 Python wheel builds, the source archive
and verified crate packaging. The workflow stores Python distributions and the `.crate` file
as artifacts. The two registries publish independently; if one succeeds and the other fails,
rerun only the failed publisher job. Already published versions cannot be overwritten.

## Installing the alpha

```sh
pip install 'tuya-interpreter==0.1.0a4'
```

```toml
[dependencies]
tuya-interpreter = "=0.1.0-alpha.4"
```

Python imports and Rust crate references use `tuya_interpreter`.
