// node bindings/js/smoke.mjs [engine.wasm]: drive a switch through the WebAssembly engine.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { Driver, Engine } from "./engine.mjs";

const root = new URL("../../", import.meta.url);
const wasm = process.argv[2] ?? new URL("rust/target/wasm32-unknown-unknown/release/tuya_rule_engine.wasm", root);
const engine = await Engine.load(await readFile(wasm));
const rules = engine.install(JSON.parse(await readFile(new URL("rules/00-default.json", root))), "default");

const device = {
  id: "plug1", category: "kg", product_id: "p", name: "Plug",
  function: { switch_1: { type: "Boolean", values: "{}" } },
  status_range: { switch_1: { type: "Boolean", values: "{}" } },
};
const plug = new Driver(engine, rules, device, { dpmap: { 1: "switch_1" } });
const published = (actions) => Object.fromEntries(
  actions.filter((a) => a.kind === "publish").map((a) => [a.topic, a.payload]));

const described = published(plug.adopt([{ type: "connected" }]));
assert.equal(JSON.parse(described["il/plug1"]).props.switch_1.type, "binary");
assert.equal(published(plug.input({ type: "message", channel: "state", json: { 1: true } }))["il/plug1/switch_1"], "true");
assert.deepEqual(plug.write("switch_1", "off"), [{ kind: "bridge", action: "set", dps: { 1: false } }]);
console.log("ok: a switch driven from JavaScript");
