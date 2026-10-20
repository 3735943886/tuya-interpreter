# Local runtime measurements

Measured on 2026-10-11 on the development Linux AArch64 host, using release builds.
These are local microbenchmarks, not throughput guarantees for other devices or hosts.
Each result is the median of five rounds after 100 warm-up messages. Construction,
rule installation and the initial state packet are excluded. Messages alternate DP 1
between `true` and `false` through the active channel.

## Python transport

Measured on 2026-10-12. `scripts/benchmark_abi.py <absolute-library-path> 2000` drives the
same messages through Python, ctypes and the JSON ABI in both ways a host can hold a driver:
explicit-state requests (`create`/`handle`) send and return the complete driver snapshot on each
message; engine-held drivers (`open`/`drive`), which `TuyaDriver` uses, send only the input and
outputs.

| Case | `handle` µs/message | `drive` µs/message | Speedup |
| --- | ---: | ---: | ---: |
| Single switch | 143.63 | 30.36 | 4.7× |
| `dj_0gyaslysqfp4gfis` light | 940.47 | 164.74 | 5.7× |

## Direct Rust calls

`cargo run --release --manifest-path rust/Cargo.toml --example benchmark -- 5000`
compares the typed in-memory API with the explicit-state JSON API in the same Rust
process. JSON calls serialize and restore the complete driver snapshot on each message;
native calls retain the driver in memory. These results measure the total API path,
including that transport difference.

| Case | Native µs/message | JSON µs/message | Native speedup |
| --- | ---: | ---: | ---: |
| Single switch | 10.15 | 139.75 | 13.76× |
| `dj_0gyaslysqfp4gfis` light | 133.41 | 936.83 | 7.02× |

Both benchmarks use the bundled default rules and the same fixture inputs. The switch
has one Boolean DP; the light is the existing multi-property conformance fixture.
Different packet mixes, converters and timer activity can produce different results.
