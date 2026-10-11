// node bindings/js/smoke.mjs [engine.wasm]: drive a switch through the WebAssembly engine.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { Driver, Engine } from "./engine.mjs";

const root = new URL("../../", import.meta.url);
const wasm = process.argv[2] ?? new URL("rust/target/wasm32-unknown-unknown/release/tuya_rule_engine.wasm", root);
const engine = await Engine.load(await readFile(wasm));
const rules = engine.install(JSON.parse(await readFile(new URL("rules/builtin/00-default.json", root))), "default");

const device = {
  id: "plug1", category: "kg", product_id: "p", name: "Plug",
  function: { switch_1: { type: "Boolean", values: "{}" } },
  status_range: { switch_1: { type: "Boolean", values: "{}" } },
};
const plug = new Driver(engine, rules, device, { dpmap: { 1: "switch_1" } });
assert.equal(plug.descriptor.props.switch_1.type, "binary");
assert.ok(plug.handle(0, { type: "connected" }).some((o) => o.type === "descriptor"));
assert.ok(plug.handle(1, { type: "message", channel: "state", json: { 1: true } })
  .some((o) => o.type === "value" && o.prop === "switch_1" && o.value === true));
assert.deepEqual(plug.handle(2, { type: "command", prop: "switch_1", value: false }),
  [{ type: "send_message", channel: "set", json: { dps: { 1: false } } }]);
assert.equal(plug.handle(3, { type: "command", prop: "missing", value: true })[0].type, "reject");
assert.throws(() => plug.handle(undefined, { type: "connected" }), TypeError);
assert.throws(() => new Driver(engine, "missing", device), /rules/);
assert.throws(() => engine.call({ op: "create", rules: { version: 1 }, device }), /rules/);
const other = JSON.parse(await readFile(new URL("rules/builtin/00-default.json", root)));
other.constants.manufacturer = "Other vendor";
engine.install(other, "other");
assert.throws(() => engine.call({ op: "handle", rules_id: "other", driver: plug.state,
  input: { type: "connected" }, now: 4 }), /do not match/);
assert.equal(engine.remove("other"), true);
assert.equal(engine.remove("other"), false);
console.log("ok: bidirectional transport-independent driver from JavaScript");
