# Interpretation behavior

This document describes the interpretation boundary and reference behavior of
`tuya-interpreter`. The [engine API and rule language](rust-engine.md) define the
implementation interfaces; the [README](../README.md) describes device input and the
ildevice output contract.

## Inputs and classification

The host supplies a decoded device object and an explicit, validated rule pack.
`function` supplies writable DP definitions, `status_range` supplies reported definitions,
`local_strategy` maps DP IDs to codes and value conversions, and `status` supplies
code-keyed initial values. Live messages contain flat DP-ID-keyed maps.

Classification applies product quirks and user overrides, then resolves roles from the
category tables and entity specifications in rule declaration order. A role chooses the
first matching candidate with an accepted type and usable definition. The selected plan
supplies the descriptor, read conversion and command behavior.

A category or a raw DP number alone does not supply missing feature definitions. Product
quirks and explicit overrides can define them. `expose_unused=True` exposes unclaimed DPs
as generic properties using available metadata; it does not infer semantic roles.

## Values and commands

DP definitions and rules determine validation, scale, units, enum mappings and codecs.
Integer scales apply powers of ten. Numeric outputs retain their published native unit;
display-unit suggestions do not rescale values. Binary ildevice properties publish JSON
booleans. Invalid or unavailable decoded values become absent rather than fabricated values.

Commands are validated against the published property contract before conversion into Tuya
DPS. Rejected commands return a structured rejection. Rounding and brightness remapping
can be lossy. Rules can select different command DPs from the DPs used for feedback.

Malformed device definitions, invalid rules and unsupported overrides are reported as
construction or validation errors. Runtime evaluation is bounded. Failed converter updates
and timers preserve driver state; failed commands publish a rejection without committing
partial converter effects. See [JSON converters](rust-engine.md#json-converters) for the
transaction and disconnect-reset rules.

## Runtime state

The host owns transport, time, timers, settings persistence and device routing. The engine
does not read the network, filesystem or clock. Each driver retains state for one device.

| Channel | Meaning |
| --- | --- |
| `active` | Live device push; updates values, emits matching events and counts increments. |
| `passive` | Readback or live report; does not emit events. Increment counting requires `delta.accept_passive`. |
| `state` | Snapshot used to seed state without replaying events or retained movement commands. |

Delta DPs with `report_type: sum` contribute increments to a running total. Repeated
increments can be distinct reports; a repeated Tuya timestamp is deduplicated. Hosts can
carry compatible totals to a replacement driver with `new.carry(old)`.

Cover positions follow the device's direction. Cover settings control inversion, position
source and motion inference. Snapshot/readback data does not establish new motion.
Disconnect clears motion, published values and timers. See
[cover and delta settings](device-customization.md) for the complete behavior.

Driver snapshots are opaque, version-specific state. Restoration requires compatible
engine state and the same ordered rule fingerprint; snapshots are not a durable storage
format. `same_config` allows a host to retain an unchanged driver during configuration reload.

## Reference behavior and intentional differences

Classification, codecs, actions and product quirks are checked against Home Assistant Tuya
fixtures and `tuya-device-handlers` reference data. SDK adapter conversions are compared
with `tuya-device-sharing-sdk==0.2.15`. These references establish conversion behavior;
ildevice descriptors and outputs follow their own contract.

The following differences are intentional:

- Tuya's standard category assignments take precedence over conflicting Home Assistant
  category classes, including switch versus socket classification.
- Cover positions follow the device's direction rather than Home Assistant's mirrored
  position convention. Hosts can configure direction explicitly.
- Binary outputs are booleans, including when a reference read accepts numeric `0` or `1`.
- Invalid encoded text, JSON or color payloads yield unavailable values rather than
  propagating reference-library decoding exceptions.
- Invalid device definitions are rejected at the interpretation boundary.

Camera streams and images, cloud authentication, scenes and transport-specific services
are outside the interpreter's scope. Camera plans can be classified without being assembled
into supported ildevice properties. Doorbell events expose their kind without a text payload.

## Validation sources

- `tests/golden/core_fixtures/` and golden snapshots cover reference classification and values.
- Quirk and action golden data cover schema patches and command conversion.
- `rust/engine/tests/driver_vectors.json` exercises complete driver traces through the JSON
  and native Rust APIs.
- Adapter tests compare SDK conversion strategies and inverse writes.
- IL conformance tests validate descriptors and commands against the specification commit
  pinned in `tests/ildevice.ref`.

Reference fixtures do not prove support for every device or firmware. A product's category,
feature definitions, conversions and any required overrides determine its supported behavior.
