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

  /** Release a registered pack only after its last driver stops using the ID. */
  remove(rulesId) {
    return this.call({ op: "remove_rules", rules_id: rulesId });
  }
}

/** One transport-independent Tuya device. Rules and time are supplied by the host. */
export class Driver {
  constructor(engine, rulesId, device, options = {}) {
    Object.assign(this, { engine, rulesId });
    const result = engine.call({ op: "create", rules_id: rulesId, device, options });
    this.state = result.driver;
  }

  get descriptor() {
    return this.state.assembly.descriptor;
  }

  /** IL driver input; returns descriptor, value, event, send_message, reject or timer data. */
  handle(now, input) {
    if (!Number.isFinite(now)) throw new TypeError("now must be a finite number supplied by the host");
    const result = this.engine.call({ op: "handle", rules_id: this.rulesId, driver: this.state, input, now });
    this.state = result.driver;
    return result.outputs;
  }

  describe(now, seed = []) {
    return this.handle(now, { type: "describe", seed });
  }

  carry(old) {
    this.state = this.engine.call({ op: "carry", rules_id: this.rulesId, driver: this.state, old: old.state }).driver;
  }

  settingsBlock() {
    return this.engine.call({ op: "settings", rules_id: this.rulesId, driver: this.state });
  }
}
