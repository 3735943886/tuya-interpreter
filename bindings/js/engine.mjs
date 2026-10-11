// A JavaScript binding: JSON through the engine's C ABI, compiled to WebAssembly
// (`cargo build --release --target wasm32-unknown-unknown`).
// It holds no Tuya knowledge; the rule pack and the engine do the interpreting.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export class Engine {
  static async load(wasmBytes) {
    const { instance } = await WebAssembly.instantiate(wasmBytes, {});
    return new Engine(instance.exports);
  }

  constructor(exports) {
    this.x = exports;
  }

  /** One request; throws on `{ok: false}`. */
  call(request) {
    const bytes = encoder.encode(JSON.stringify(request));
    const len = bytes.length + 1;
    const input = this.x.tuya_engine_alloc(len);
    try {
      new Uint8Array(this.x.memory.buffer, input, len).set(bytes);
      const output = this.x.tuya_engine_eval(input);
      try {
        const memory = new Uint8Array(this.x.memory.buffer);
        let end = output;
        while (memory[end] !== 0) end++;
        const response = JSON.parse(decoder.decode(memory.subarray(output, end)));
        if (!response.ok) throw new Error(response.error);
        return response.value;
      } finally {
        this.x.tuya_engine_free(output);
      }
    } finally {
      this.x.tuya_engine_dealloc(input, len);
    }
  }

  /** Register a rule pack once; later requests name it. */
  install(rules, rulesId) {
    this.call({ op: "install_rules", rules_id: rulesId, rules });
    return rulesId;
  }
}

/** One Tuya device behind an il-mqtt producer: its driver state lives here as JSON, between calls. Every method
 *  returns host actions (`publish`, `schedule`, `unschedule`, `bridge`, `save_settings`). */
export class Driver {
  constructor(engine, rulesId, device, options = {}, topics = {}) {
    Object.assign(this, { engine, rulesId, topics });
    this.state = engine.call({ op: "create", rules_id: rulesId, device, options }).driver;
  }

  hub(action, fields) {
    const result = this.engine.call({ op: "hub", rules_id: this.rulesId, topics: this.topics, action, ...fields });
    this.state = result.driver ?? this.state;
    return result.actions;
  }

  /** Publish the device (`seed`: what is already known of it, e.g. `[{type: "connected"}]`). */
  adopt(seed = [], now = Date.now() / 1000) {
    return this.hub("adopt", { old: null, new: this.state, seed, now });
  }

  /** Bridge input (`{type: "connected"}`, `{type: "message", channel, json}`, ...) or `{type: "timer", name}`. */
  input(input, { retained = false, now = Date.now() / 1000 } = {}) {
    return this.hub("input", { driver: this.state, input, retained, now });
  }

  /** An il write to `prop` (the payload of `il/<id>/<prop>/set`). */
  write(prop, payload, now = Date.now() / 1000) {
    return this.hub("write", { driver: this.state, prop, payload, now });
  }
}
