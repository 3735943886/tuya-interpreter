# Local runtime measurements

Measured on 2026-10-11 on the development Linux AArch64 host, using release builds.
These are local microbenchmarks, not throughput guarantees for other devices or hosts.
Each result is the median of five rounds after 100 warm-up messages. Construction,
rule installation and the initial state packet are excluded. Messages alternate DP 1
between `true` and `false` through the active channel.

## Existing Python transport

`scripts/benchmark_abi.py <absolute-library-path> 2000` uses the same Python,
ctypes and JSON transport for both revisions. The baseline is commit `8183033`;
the updated library is built from the accompanying working-tree changes.

| Case | Baseline µs/message | Updated µs/message | Time reduction |
| --- | ---: | ---: | ---: |
| Single switch | 146.86 | 139.94 | 4.7% |
| `dj_0gyaslysqfp4gfis` light | 1,412.81 | 933.23 | 33.9% |

## Direct Rust calls

`cargo run --locked --release --manifest-path rust/Cargo.toml --example benchmark -- 5000`
compares the updated typed in-memory API with the updated JSON API in the same Rust
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
