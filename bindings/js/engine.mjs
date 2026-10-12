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

/** Releases engine-held drivers that were collected without `close()`. */
const released = new FinalizationRegistry(({ engine, handle }) => {
  try {
    engine.call({ op: "close", handle });
  } catch {
    // The engine instance may already be gone.
  }
});

/**
 * One transport-independent Tuya device. Rules and time are supplied by the host.
 * Its state stays in the engine; each input sends only itself and its outputs.
 */
export class Driver {
  constructor(engine, rulesId, device, options = {}) {
    Object.assign(this, { engine, rulesId });
    this.handleId = engine.call({ op: "open", rules_id: rulesId, device, options }).handle;
    this.status = { linked: false, synced: false, timers: [] };
    released.register(this, { engine, handle: this.handleId }, this);
  }

  /** A version-specific snapshot of the whole driver, for inspection and migration. */
  get state() {
    return this.engine.call({ op: "snapshot", handle: this.handleId });
  }

  get descriptor() {
    return this.engine.call({ op: "view", handle: this.handleId }).descriptor;
  }

  get linked() {
    return this.status.linked;
  }

  get synced() {
    return this.status.synced;
  }

  /** IL driver input; returns descriptor, value, event, send_message, reject or timer data. */
  handle(now, input) {
    if (!Number.isFinite(now)) throw new TypeError("now must be a finite number supplied by the host");
    const { outputs, ...status } = this.engine.call({ op: "drive", handle: this.handleId, input, now });
    this.status = status;
    return outputs;
  }

  describe(now, seed = []) {
    return this.handle(now, { type: "describe", seed });
  }

  carry(old) {
    this.engine.call({ op: "carry_handle", handle: this.handleId, old: old.handleId });
  }

  settingsBlock() {
    return this.engine.call({ op: "settings", rules_id: this.rulesId, driver: this.state });
  }

  /** Release the engine's copy of this driver. */
  close() {
    released.unregister(this);
    return this.engine.call({ op: "close", handle: this.handleId });
  }
}
