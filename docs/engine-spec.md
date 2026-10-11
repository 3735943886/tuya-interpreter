# Tuya DP engine — specification (draft 2)

> The engine is `rust/engine/` driven by the rule pack `rules/builtin/00-default.json` (this draft predates the move; its
> platform builders are now the pack's `entities` specs). "IL" and "tuya2ha" below mean this engine's own model,
> **not** the ildevice IL (which the top-level package produces from the engine's entity plans).

Goal: a sans-I/O, dependency-free description of *everything* HA core's official `tuya`
integration (dev @949a484 + tuya-device-handlers 0.0.29) does, so any host can reproduce
it exactly by interpreting data. Target = exact behavioral reproduction incl. core's own
quirks (section 12). tuya-device-handlers is a TEST ORACLE only.

Non-goals: cloud auth/MQ, scenes, camera *stream/image* (entity shape is in scope, B6),
feeder services (K12), user Python quirks, entity-level `*Quirk.definition_fn` (unused in
0.0.29), lock/media_player/water_heater (core has none), host rendering (topics, HA
classes, translations — section 10).

## 0. Design rules
R0.1 **Data + closed algorithms, no code hooks.** Everything core does is table data, a
*named op* from a closed registry, or an `Expr`/`Cond`. New kind of behavior = add one op
or construct (additive, versioned); never reshape models.
R0.2 **Runtime read/write is keyed by dp code, never by dp id.** dp ids appear only in
`DpMap` (adapter input) and quirk `DefineDp/RemoveDp` (which create/remove dpmap entries).
R0.3 **Pure functions over explicit inputs** (signatures in section 7 are normative);
only mutable object is the explicit per-entity `StateSlot`.
R0.4 **Reads never raise**: invalid/absent raw values are `UNKNOWN` (== Python `None`).
Writes reject with `WriteRejected` (never clamp). Core paths that *raise* on bad data are
enumerated in section 12 with an explicit tag `reproduce | deviate`; every deviation is
listed there — there are no untagged raise paths.
R0.5 **Literals are data** ("FZ","ZZ","STOP","chargego","back","Sensor Low Battery",
"white","colour"): only as op/expr arguments.
R0.6 **Platform vocabularies are typed and fixed** (section 8). Free-form role names are
forbidden.
R0.7 **Versioned:** `IL_VERSION = 2` (additive minor bumps); tables/quirks carry the
core commit + IL version they target.
R0.8 **UNKNOWN semantics** (normative for all `Expr/Cond/Pipeline`): `UNKNOWN == None`;
comparisons follow Python on `None` (`Ne(None,"back")` is True; `Eq(None,x)` False for
x≠None; `Ge(None,n)` False); `Contains(None,x)` False; `Decode(None)` -> None; any op
given UNKNOWN returns UNKNOWN (read) — except `Truthy/IsNone/Is/OrElse/Override`, which
inspect it. `Ge/Gt/Le/Lt` on a non-number = False.

## 1. Inputs

### 1.1 DpType / DpSpec
```
DpType = Bitmap | Boolean | Enum | Integer | Json | Raw | String
normalize_type(s) -> DpType|None   # exact names + lowercase aliases:
   bitmap,bool,enum,json,raw,string,value(->Integer)  (const.py:44-63)
   NOT "Bool"/"Value"; upper-case "ENUM"/"STRING"/"BOOLEAN" (present in fixtures) are
   UNPARSED => that dp resolves to nothing (parity; oracle does the same)
DpSpec{code, type, values: dict|str (verbatim), report_type|None}
SpecInteger{min,max,scale,step:int (via int(): truncates floats); unit|None}  # all 4 REQUIRED
SpecEnum{range:list[str]}  SpecBitmap{label:list[str]}
SpecJson{h,s,v:{min,max}}  # accessor used ONLY by hsv ops; parsed from Json dp `values`
```
`values` accepted as dict or JSON string. falsy `values` (`{}`/`null`/"") => spec "not
found" for Enum/Bitmap/Integer. Malformed (bad JSON, missing required key): see P-13.

### 1.2 DeviceSchema and env
```
DeviceSchema{ id,name,category,product_id,product_name, online,
  function: dict[code,DpSpec],  status_range: dict[code,DpSpec],
  status: dict[code,raw],       # live values; needed by classify (probes, When)
  type_overrides: dict[code, "invert_int_max"],   # set by quirks; consulted by resolve()
  dpmap: dict[dpid:int, code] } # ADAPTER-owned; quirks may add/remove entries
HostEnv{ temperature_unit: "C"|"F",
         allowed_units: {sensor|number: {device_class: [unit...]}} }  # HA's
         # SENSOR_/NUMBER_DEVICE_CLASS_UNITS, supplied by the host (5.3)
```
`status` values follow core's contract: already *cloud-converted* (bool/int/str; Raw =
base64 str; Json = JSON str). Producing that from bridge-raw values is the adapter's job.

**Schema synthesis (A1/I9).** classify needs `function`/`status_range` with types and
ranges. For registered devices the host supplies the cached cloud record (rustuya-local
already stores function/status_range/local_strategy). A device with no cached schema
gets NO entities (core parity: nothing to classify); the IL does not invent schemas from
`local_strategy` or categories. (Static per-product schemas exist only through quirks.)

### 1.3 Quirks = schema patches
```
QuirkSet{ product_id (exact, case-sensitive; ONE per product_id, later replaces),
          meta{manufacturer,model,model_id}, ops: ordered list[QuirkOp] }
QuirkOp (each with optional `when: Cond` over Status/Lit/IsInt/compare/And/Or/Not):
  SetCategory(category)
  DefineDp(dpid, code, type, mode: R|W|RW, values, report_type|None)
     # FULL REPLACE: READ -> status_range[code]=..., else pop; WRITE -> function[code]=...,
     # else pop; ALWAYS sets dpmap[dpid]=code (core: local_strategy only if support_local
     # — irrelevant for a local IL, M4). Builders exist for Bitmap/Boolean/Enum/Integer only.
  RemoveDp(dpid, code)              # function, status_range, status, dpmap
  MapInitialStatus(code, {raw: value})   # see apply_status_quirk below
  TypeOverride(code, "invert_int_max")   # -> schema.type_overrides[code]; matched by
                                         # dpcode only, dpid ignored (core parity)
  DpValueMapEnum(dpid, {n: v})      # ADAPTER hint (local enum map, K11) — not interpreted
apply_quirk(schema, QuirkSet) -> schema'          # pure; BEFORE classify; ops in order
apply_status_quirk(quirk, status) -> status'      # host calls it on the INITIAL status
   # only (core applies MapInitialStatus once at init; later pushes arrive converted).
   # It is NOT part of read() (I6).
device_info(schema, quirk) -> {manufacturer, model, model_id}   # util.py:65-87
   # default "Tuya"/product_name/product_id; if quirk.manufacturer truthy ALL THREE are
   # the quirk's (manufacturer-only => model/model_id None; P-11)
```
`invert_int_max` semantic (P-06): for every role reading that code, inserted between
`validate_int` and any downstream `remap`: read `scale(max) - v`, write `scale(max) - v`
(inverts about 0..max, ignoring min). Composition order is fixed: validate_int ->
invert_int_max -> remap -> round (read); reverse for write.

## 2. DP resolution
```
DpRef{ candidates: tuple[code], types: tuple[DpType]  (usually 1),
       source: "status_range_first" | "function_first" }
resolve(schema, DpRef) -> ResolvedDp{code, spec, kind: DpType, report_type}|None
```
= `TypeInformation.find_dpcode` (type_information.py:66-118): code-major; per candidate
try sources in order; accept iff entry exists AND `normalize_type(type) in types` AND spec
parses. `report_type` from `status_range[code]` ONLY (None if code is function-only).
Wrong-typed entries skipped silently. (`multi_pass` is deleted: fan speed's "Integer over
all candidates, then Enum" is two role alternatives, section 6.)

## 3. Expressions (normative vocabulary; semantics per R0.8)
```
Expr:  Lit(v) | Tuple(e...) | Index(e, i) | Arg(name) | Derived(field) | Status(code)   # raw live value
       Resolved(role) | Raw(role)     # Resolved = value after read pipeline; Raw = validated
                                      # raw value before mapping ops (vacuum status)
       SpecOf(role, min|max|step|scale|unit|range|labels)   # bound spec metadata
       Decode(e, b64|utf8|utf16be)  Map(e, table, default)  OrElse(a, b)  # a if truthy else b
       Round(e) | Not-truthy helpers as Cond
Cond:  Eq Ne In Ge Gt Le Lt (e, e)  IsInt(e) IsNone(e) Is(e, lit)   # identity: `is False`
       Truthy(e)  Contains(text_e, substr)  Member(e, list_e)
       Has(code, where: function|status_range|status|any)  Found(role)  AltIs(role, i)
       Kind(role, DpType)  TypeIs(code, DpType)  InRange(role, lit)  ArgPresent(name)
       And Or Not
StateExpr: FirstOf([e...])        # VALUE fallback: first non-UNKNOWN
           PreferRole(a, b)       # PRESENCE fallback at resolution: a if Found(a) else b
                                  # (cover current_position or set_position)
           Override(cond, v, else_)  Table(e, map)  Choose([(cond, expr)...], else_)
ListExpr:  Options(role) | Filter(list, cond) | Append(list, e) | OfPresent([(cond, e)...])
```
Use sites and legal subsets: quirk `when` (Status/Lit/IsInt/compares/And/Or/Not/Has),
`remap.reverse` (Cond over Status), StateExpr/`derived`, existence (Has/TypeIs/Found), write
`when`/values (everything incl. Arg/Derived). No loops, no arithmetic beyond op params.

## 4. Existence (per role alternative or per entity)
```
Existence = TypedDp(DpRef) | AnyName(codes, where=any)   # name in function|status|status_range
          | Always | KeyPresent(code)                    # any type, function|status_range
          | BitmapLabel(DpRef(Bitmap), label) | ParsedAttr(DpRef(Raw|Json), codec|attr)
            # found iff no payload yet (falsy decoded value / status None) OR parsed attr
            # not None / key in dict (needs status)
          | All(list) | AnyOf(list)   # AnyOf = FIRST matching alt in list order
```
Selection: `tables[platform][category]` -> ordered `EntityDef` list; unknown category =>
none; post-quirk category. **Aliases are per platform** (I8): light cz=kg, pc=kg,
dghsxj=sp, tdq=tgq; switch cz=pc, dghsxj=sp; sensor dghsxj=sp, pc=kg; select cz=kg, pc=kg,
dghsxj=sp; number dghsxj=sp; siren dghsxj=sp; camera dghsxj=sp. Shared tuples
(`BATTERY_SENSORS` x35, `TAMPER` x17) are generator expansions. One EntityDef may `expand`
into N entities (indexed / per-enum-value; keys verbatim, incl. inconsistent ones).
Several EntityDefs may share a dp; no `consumed` set. Binary-sensor branches are
EXCLUSIVE (`AnyOf`, first alt wins; `bitmap_key` set => bitmap only, no fallback).

## 5. Pipelines
`Pipeline` = ordered ops; each op: `read(v)->v|UNKNOWN`, optional inverse `write(v)->raw`
(may raise `WriteRejected`). Bound to spec metadata at classify (no schema access at
runtime). UNKNOWN short-circuits reads.

| op (params) | read | write | ev. |
|---|---|---|---|
| `validate_int` | int (bool ok, float rejected) in [min,max] -> `/10**scale`; else UNKNOWN | numeric; `round(v*10**scale)` banker's; in [min,max]; step NOT enforced | T2,T3 |
| `validate_enum` | in range else UNKNOWN | str in range | T5 |
| `validate_bool` | `raw in (True,False)` (0/1 pass as-is, P-12) | real bool | T6 |
| `validate_bitmap` | int passthrough | - | T11 |
| `parse_json` `b64_decode` `passthrough` | json.loads / non-strict b64 / verbatim; failure UNKNOWN (P-14) | - | T12,T14 |
| `invert_bool` | `not v` | validate then `not v` | T7 |
| `invert_int_max` (from `type_overrides`) | see 1.3 | same | T8 |
| `remap(src, dst, reverse, round)` | linear `((v-a)/(b-a))*(d-c)+c`; `src`/`dst` endpoints are `Expr`s (default: spec scaled [min,max] -> target); `reverse` is `bool` or `Cond` (cover `control_back_mode`); `round` banker's | inverse then `validate_int` write | T9 |
| `round_int` | `round(v)` | via validate_int | T10 |
| `json_attr(name)` | key of parsed dict | - | T13 |
| `parsed_attr(codec, attr)` codec: electricity_raw/electricity_hex; meta{native_unit,suggested_unit} | `getattr(parse(v), attr)` | - | T15-17 |
| `bit_test(label)` | `(raw & 1<<labels.index(label)) != 0` | - | |
| `in_set(values)` / `eq(x)` | raw compare (Raw compared as base64 str) | - | |
| `map_table(t, missing=UNKNOWN)` | many-to-one | inverse iff declared bijective | |
| `enum_map_filtered(t)` | write inverse only: first raw whose *filtered* target == x (strings sharing a target ALL become unmapped) | | climate |
| `enum_map_unfiltered(t)` | read (hvac_mode, P-02): plain lookup | | climate |
| `enum_unmapped_options(t)` | presets: read = raw if in options else UNKNOWN; options = strings unmapped after filtering | write validate_enum | climate |
| `enum_position_percent` | `(idx+1)*100 // n` | first option with `(pos*100)//n >= p`, else last | fan |
| `mired_kelvin(range=2000..6500)` | reversed linear over mired, `round(1e6/mired)` | inverse | light |
| `hsv_json(ranges)` / `hsv_hex(ranges)` | tuple (h,s,v) unrounded; hex read only if len==12 hex; json missing key => UNKNOWN (P-22) | `json.dumps({"h":..,"s":..,"v":..})` DEFAULT separators `{"h": 10, "s": 202, "v": 1000}` / `f"{h:04x}{s:04x}{v:04x}"` | light |
| `temp_convert(from,to)` | C<->F | inverse | climate |
| `decode(kind)` | text; failure UNKNOWN (P-14) | - | alarm,event |
| `delta_accumulate` | stateful (7.3) | - | sensor |

`hsv ranges` = `FromSpec | V1 | V2`; from-spec: h,s,v `SpecJson` min/max (defaults h 0-360,
s 0-255, v 0-255) -> dst (0-360, 0-100, 0-255); V1 = H(1..360),S(1..255),V(1..255); V2 =
H(1..360),S(1..1000),V(1..1000) (all -> 0-360/0-100/0-255). Fallback used when spec empty
or dp is String; V2 iff `Or(desc.v2_flag, DpcodeIs("colour_data_v2"), Gt(SpecOf(brightness,
max),255))` (B7a).
**Brightness limits (B7b):** `remap` endpoints are Exprs, so limits = "if Found(bmin) and
Found(bmax) and both Resolved non-UNKNOWN: src=[remap255(bmin), remap255(bmax)] else
src=spec range" — read and write.

### 5.3 UnitPolicy (B3; REQ-RD-23) — classify-time op for sensor and number
Inputs: `dp_unit` (spec unit / wrapper `native_unit`), `desc{device_class, native_unit_fallback,
suggested_unit}`, wrapper `suggested_unit`, `Status("temp_unit_convert")`,
`env.allowed_units[platform][device_class]`, alias table `units.py` (C/const.py:1034-1229,
~35 rows; RENAME only, no arithmetic). Steps (sensor.py:1879-1936, number.py:577-629):
1 device_class ENUM -> unit None. 2 device_class None, or dp_unit in allowed -> unit=dp_unit.
3 device_class TEMPERATURE, dp_unit empty, `temp_unit_convert` in {c,f} -> °C/°F. 4 alias
lookup (exact then lowercase) -> canonical unit if it is allowed. 5 else if
`native_unit_fallback` -> use it. 6 else unit=dp_unit and DROP device_class and
suggested_unit. Promotions: `device_class None + Enum alt` => ENUM (options=range, unit
None); `delta` alt => `state_class` TOTAL_INCREASING if unset. Output
`{native_unit, device_class|None, suggested_unit|None, state_class, options}`.
The wind-direction op has no unit => device class dropped (snapshot: None/None).

## 6. Entities, roles, derived, actions, features
```
EntityDef{ platform, key (verbatim, unique-id suffix), identity{translation_key,
  placeholders, name|None, entity_category, device_class, state_class, enabled_default,
  icon, native_unit_fallback, suggested_unit, precision}, expand?, existence: Existence,
  roles: dict[RoleName, Role], derived, actions, features, update: own_dps|any_update,
  state_slot: none|delta }
Role{ required: bool, alts: [Alt] }        # B1 — resolution = FIRST alt that matches
Alt{ dpref: DpRef, exists: Existence|None, read: Pipeline, write: Pipeline|None,
     meta{native_unit, suggested_unit, options_from: role|None} }
ResolvedRole{ alt_index, kind, code, spec }   # AltIs/Kind conds read this
```
Alternatives express: fan speed (alt0 Integer -> remap 1..100, alt1 Enum ->
enum_position_percent); cover instruction (enum map | special FZ/ZZ/STOP map | boolean);
sensor (Integer+`report_type==sum` -> delta | Integer | Enum); electricity (Raw | Json);
colour (Json | String); binary_sensor (bitmap | Boolean | legacy in-set). A role with zero
matching alts is absent (`Found(role)` False).
**StateExpr rules:** `FirstOf` is value fallback; presence fallback is `PreferRole`;
vacuum activity is `Override(Not(IsNone(Raw(status))), Table(Raw(status), map),
Override(Truthy(Raw(pause)), PAUSED, UNKNOWN))` (status present-but-unmapped => UNKNOWN,
pause NOT consulted; the map is applied in `derived`, not in the role pipeline).
### 6.2 WriteProgram
```
WriteProgram(args: [ActionArg], body: [Stmt])
Stmt = Step | Choose | FirstApplicable
Step(when: Cond|None, role, value: Expr, if_missing: skip | Raise(ActionDPCodeNotFound,
     expected=[..]))                                  # default skip (I3)
RawStep(code: Expr, value: Expr)                      # vacuum send_command (params non-empty)
Choose([(Cond, [Stmt])...], else_=[])                 # exclusive if/elif
FirstApplicable([Program...])   # first whose required roles are all Found and guards true
```
All Conds/Exprs are evaluated against the state BEFORE the action (no intra-program
mutation); each Step yields `{"code","value"}` via the role's write pipeline (may raise
`WriteRejected`); result = ordered list; empty list = no-op; batching = host.
### 6.3 Features and lists
`FeatureRule(flag <- Cond)` evaluated at classify over `Found/Kind/AltIs/InRange`;
`ListExpr` builds `hvac_modes`, `preset_modes`, swing options, `available_modes`,
`fan_modes`, `event_types`, `fan_speed_list`. Light color modes: named op
`filter_color_modes(set)` (HA ONOFF/BRIGHTNESS pruning, `_fixed_color_mode`) +
`white_mode` default COLOR_TEMP (WHITE only via: no color_temp, HS present, "white" in
work_mode range).
### 6.4 Climate temperature selection (B2; REQ-RD-19) — classify-time op
`select_temp_roles(candidates, alias_sets, host_unit)`: (1) if a `temp_unit_convert` enum
resolves: every resolved temp role with EMPTY unit gets unit = its RAW status string
(read ONCE, P-05); (2) classify each of cur_c/cur_f/set_c/set_f by `unit.lower()` in
C aliases {°c,c,celsius,℃} / F aliases {°f,f,fahrenheit,℉}; (3) if host F and
`(curF&setF)|(curF&!setC)|(setF&!curC)` -> F pair, entity unit F; elif
`(curC&setC)|(curC&!setF)|(setC&!curF)` -> C pair, entity unit C; else fallback: host F ->
`(cur_f or cur, set_f or set, F)` else `(cur or cur_f, set or set_f, C)`. Output: chosen
(current, set, entity_unit) + per-role native unit feeding `temp_convert` on read/write.
min/max/step stay in the DP's NATIVE unit (P-04). Default step 1.0.
### 6.5 Camera (B6)
`camera` platform: existence `Always` for sp/dghsxj; roles `motion_switch` (bool,
function_first, rw), `record_switch` (bool, status_range_first, r); derived `is_recording`,
`motion_detection_enabled` = `OrElse(Resolved(motion_switch), False)`; actions
enable/disable_motion_detection. Stream/image = adapter.

## 7. Runtime API (as implemented in `tuya/runtime.py` + `tuya/platforms.py`)
```
classify(schema, env=None, platforms=None) -> Plan            # platforms: which tables to build, default all
   Plan{entities: [EntityPlan]}
   EntityPlan{platform, key, identity, roles: {name: ResolvedDp}, depends_on: (code...),
              read, write, slot_kind: None|"delta"|"event", update_all: bool}
```
`read` and `write` are not standalone functions taking `(EntityPlan, ...)`: each platform builder in
`platforms.py` returns a closure already bound to that entity's resolved dps, spec and `env` (env is
consulted only at classify time, e.g. unit policy; nothing keeps it at runtime). Calling convention:
`read(status) -> dict[field, value]` for every platform except `slot_kind == "delta"`, which is
`read(status, slot) -> dict` (`assemble.py` branches on `plan.slot_kind` to pick the arity).
`write(action, args, status) -> list[{"code","value"}]`, may raise `WriteRejected`. There is no
`available`/`State` wrapper or `schema.online` field inside the engine; availability is computed by
the host (`TuyaDriver`) outside `tuya/`, from the packet, not by `read`. There is no `reclassify`;
call `classify()` again with the new schema (it is a pure function of `schema, env, platforms`).
```
new_slot(plan) -> StateSlot|None                               # StateSlot() iff plan.slot_kind, else None
on_update(plan, slot, changed|None, dp_timestamps|None, status) -> UpdateResult{write_state, fire}
   # fire: tuple[event_type, attrs|None] | None (event platform only)
```
`depends_on` = role codes + every `Status`/`Raw` reference (cross-dp). `on_update`:
`changed is None` (online/offline/name) => `write_state=True`, never accumulates/fires;
`own_dps` platforms => `write_state` iff any dependency in `changed`; `any_update` =>
True; **event** => `fire` iff own code in `changed` AND value truthy (repeats fire; initial
status never fires); **delta** (7.3). `Command = {code, value}`; code->dpid is the adapter.
7.3 Delta (`state_slot: delta`, chosen by alt `Integer + status_range.report_type=="sum"`):
`StateSlot{total=0.0,last_ts=None}` (not persisted; starts at 0 even if status None). On
update with own code in `changed`, `dp_timestamps` not None and has the code, `ts != last_ts`
and validated value not None: `total += float(scaled)`, `last_ts=ts`, write_state; else
no write. `read` returns `total`. Timestamps: host-supplied (Q4).

## 8. Platform schemas (fixed vocabularies)
(roles / derived / actions / feature flags; the machine form is `tuya/platforms.py`)
- switch: main | is_on | turn_on/off | -
- binary_sensor: main(alts: bitmap/bool/in-set) | is_on | - | -
- sensor: main(alts) (+parsed attrs) | native_value,unit,device_class,options | - | -
- number: main:int | value,min,max,step,unit | set | -
- select: main:enum | current,options | select | -
- button: main:bool | - | press(True) | -
- event: main(alts: enum/b64str/b64raw) | last_event | - | event_types
- cover: instruction, current_state, current_position, set_position, tilt | is_closed,
  position,tilt | open/close/stop/set_position/set_tilt | OPEN,CLOSE,STOP,SET_POSITION,SET_TILT
- light: switch,brightness,bmin,bmax,color_temp,color_data,work_mode | color_mode,
  brightness,hs,kelvin,supported_color_modes | turn_on(white,color_temp_kelvin,hs_color,
  brightness)/turn_off | color modes
- climate: cur_temp(c,f),set_temp(c,f),unit_convert,cur_hum,set_hum,fan,hvac_mode,preset,
  swing{on_off,h,v},switch | hvac_mode,hvac_modes,presets,temps,swing | set_temperature/
  hvac_mode/preset/fan/swing/humidity/turn_on/off | flags
- fan: switch,speed(alts),mode,oscillate,direction | is_on,percentage,speed_count |
  turn_on/off/set_percentage/preset/oscillate/direction | flags
- humidifier: switch,cur_hum,target_hum,mode | is_on,humidity,modes,min/max(0/100 default) |
  on/off/set_humidity/set_mode (missing dp => Raise ActionDPCodeNotFound) | MODES
- vacuum: status,pause,suction,switch_charge,seek,mode,power_go | activity | start/stop/
  pause/return_to_base/locate/set_fan_speed/send_command(RawStep) | flags
- valve, siren: main:bool | is_closed / is_on | open,close / turn_on,off | -
- alarm_control_panel: master_mode, alarm_msg (+ `Status("master_state")`) | alarm_state,
  changed_by | arm_home/arm_away/disarm/trigger | flags
- camera: 6.5
`update:` own_dps = valve,siren,binary_sensor,sensor,number,select,switch,event; any_update
= cover,vacuum,alarm,light,climate,fan,humidifier,camera; button: n/a (stateless).

## 9. Data layers (I5)
L1 **generated** from core (`TuyaXxxEntityDescription` literals + shared-tuple expansion +
aliases): pinned to a core commit, regenerated by script, hand edits forbidden.
L2 **hand-authored, reviewed** platform definitions mirroring handlers `definition/*.py`:
roles/alts/dpcode tuples and the *wrapper-class -> pipeline* table the generator resolves
(`CoverClosedEnumWrapper`->map_table, `DPCodeInvertedPercentageWrapper`->remap reverse...).
L3 quirks (`quirks.json`, 32, data; generated/verified against handlers).
Layout: the rule pack `rules/builtin/00-default.json` (`platforms` = L1, `entities` = L2, `quirks` = L3, `host_units`) and
the interpreter in `rust/engine/src/`.

## 10. Adapter boundary (not IL)
dpid<->code, SDK value-convert read+write (key = `statusFormat` key), bridge raw-value
normalization (float ints, enum wire types — LIVE VERIFY), timestamps, host units + HA
allowed-unit tables, online, translations/`strings.json` (names, state labels), icons,
HA/MQTT rendering, unique_id format (reference `tuya.{device_id}{key}`, no separator;
prefix is host-side, M8), stale-registry cleanup, fixture-loader normalization (11).

## 11. Testing
T1 oracle: for each core fixture (324, `no_quirk`) classify == core's 1205 snapshot
entities (platform, unique_id suffix, features, key attrs, state). Loader rules: Json dict
values re-serialised to strings, "**REDACTED**" -> "", device id = reversed
`{category}{product_id}` stripped of `_`; a test asserts upper-case-type DPs resolve to
nothing. T2 quirks-on matrix vs handlers oracle. T3 op round-trip property tests (allowing
declared lossiness P-10). T4 one test per parity item (section 12). T5 dpid paths via the 16
`local_strategy` fixtures + synthetic.

## 12. Parity table — core behaviors, IL behavior, tag
| id | core behavior | IL | tag |
|---|---|---|---|
| P-01 | climate entity for every dbl/kt/qn/rs/wk/wkf device (Always) | same | reproduce |
| P-02 | hvac_mode read uses UNFILTERED map; may be absent from hvac_modes; `switch is False`->OFF (identity) | `enum_map_unfiltered` + `Is(...,False)` | reproduce |
| P-03 | cover open/close write ONLY position when set_position exists; read position = current or set (by presence) | `FirstApplicable`, `PreferRole` | reproduce |
| P-04 | climate min/max/step in native unit | 6.4 | reproduce |
| P-05 | temp_unit_convert read once at classify | 6.4 | reproduce |
| P-06 | inversion about max ignoring min | `invert_int_max` | reproduce |
| P-07 | vacuum always for sd; climate appends switch_only_hvac_mode when presets exist | Always / `Append` | reproduce |
| P-08 | light: brightness DP-min -> HA 0; no work_mode => white; non-"white" => HS; one-of-two limits ignored; `_white_color_mode` default COLOR_TEMP | 5, 6.3 | reproduce |
| P-09 | swing options never list BOTH; oscillate = first found of h/v | `OfPresent` | reproduce |
| P-10 | remap round-trip lossy (banker's both ways) | `remap` | reproduce |
| P-11 | manufacturer-only quirk => model/model_id None | `device_info` | reproduce |
| P-12 | bool read leaks 0/1 | `validate_bool` | reproduce |
| P-13 | malformed `values` (bad JSON/missing key/`int()` truncation) raises out of find_dpcode | `SchemaError` at classify (whole device) | **deviate** (pending Q3) |
| P-14 | `alarm_msg`/event b64+utf8/utf16 decode failure raises; `json.loads(dict)` TypeError | UNKNOWN | **deviate** (pending Q3) |
| P-15 | int read rejects float; bool counts as int; step never enforced | `validate_int` | reproduce |
| P-16 | alarm changed_by ignores low-battery exclusion; DISARM raises if not in range | derived / Raise | reproduce |
| P-17 | fan percentage==0 + switch -> switch off; no switch -> speed write; turn_on w/o switch no-op; speed 1..100 | programs | reproduce |
| P-18 | binary legacy existence = name in function|status|status_range; humidifier/fan any-type | `AnyName` | reproduce |
| P-19 | event fires on updates containing dp, truthy value; repeats fire; initial never | `on_update` | reproduce |
| P-20 | cover tilt for EVERY cover description via its (inverted) pipeline | role `tilt` | reproduce |
| P-21 | vacuum status present-but-unmapped => unknown (pause not consulted) | Override on Raw | reproduce |
| P-22 | color JSON read: missing h/s/v raises KeyError; hex read requires len 12 | UNKNOWN | **deviate** (pending Q3) |
| P-23 | light turn_on falsy fallbacks (`kwargs.get(B)` falsy -> `self.brightness or 0`; hs or (0,0)); switch always first | `OrElse` | reproduce |
| P-24 | fan direction unknown write silently ignored; read needs truthy raw | program | reproduce |
| P-25 | humidifier missing control dp => ActionDPCodeNotFound(expected, available) | `Raise` | reproduce |
| P-26 | delta: `changed is None` bypasses (no accumulate); not persisted; starts 0 | 7.3 | reproduce |
| P-27 | upper-case type names unparsed | `normalize_type` | reproduce |
| P-28 | core tags a `kg` (switch) category's switches `SwitchDeviceClass.OUTLET` (a plug icon); Tuya's own category list names `kg` "Switch", `cz` "Socket", `pc` "Power strip" | `switch` device class by the standard's category name (`tuya/standard.py`, data in `tables/_tuya_categories.json` + `_tuya_standard_rules.json`); only a socket-like class core sets is replaced | **deviate** (Tuya's standard wins over core) |
Unverified in core (no fixtures): cover tilt, `mach_operate`; reproduced from code.

## 13. Appendix — expressiveness proofs (draft-2 constructs)
**A. Light turn_on (light.py:484-545)** — inputs white, color_temp_kelvin, hs_color, brightness:
```
Step(role=switch, value=Lit(True))
Step(when=And(Found(work_mode), Or(ArgPresent(white),ArgPresent(color_temp_kelvin))),
     role=work_mode, value=Lit("white"))                        # write validates "white" in range
Step(when=And(Found(color_temp), ArgPresent(color_temp_kelvin)), role=color_temp,
     value=Arg(color_temp_kelvin))
Choose([
 (And(Found(color_data), Or(ArgPresent(hs_color),
      And(ArgPresent(brightness), Is(Derived(color_mode),HS),
          Not(Member(WHITE,Derived(supported_color_modes))),
          Not(Member(COLOR_TEMP,Derived(supported_color_modes)))))),
  [Step(when=Found(work_mode), role=work_mode, value=Lit("colour")),
   Step(role=color_data, value=Tuple(
        Index(OrElse(Arg(hs_color),OrElse(Derived(hs_color),Lit((0,0)))),0),
        Index(OrElse(Arg(hs_color),OrElse(Derived(hs_color),Lit((0,0)))),1),
        OrElse(Arg(brightness),OrElse(Derived(brightness),Lit(0))))) ]),
 (And(Found(brightness), Or(ArgPresent(brightness),ArgPresent(white))),
  [Step(role=brightness, value=OrElse(Arg(brightness),Arg(white)))])])
```
**B. Cover**: open = `FirstApplicable([Program(Step(role=set_position,value=Lit(100))),
Program(Step(role=instruction,value=Lit(OPEN)))])` (P-03). Position =
`Resolved(PreferRole(current_position,set_position))`; `is_closed` =
`Choose([(Not(IsNone(Derived(position))), Eq(Derived(position),0))],
Table(Resolved(current_state), closed_map))`. Inversion on the position pipeline:
`remap(reverse=Ne(Status("control_back_mode"),"back"))` (missing status => inverted).
**C. Vacuum activity**: see 6. **D. Alarm**: `Override(And(Eq(Status(master_state),"alarm"),
Not(Contains(Decode(Decode(Status(alarm_msg),b64),utf16be),"Sensor Low Battery"))),
TRIGGERED, Table(Resolved(master_mode), map))`. **E. Swing read**: `Choose([(Truthy(on_off),ON),
(And(Truthy(h),Truthy(v)),BOTH),(Truthy(h),HORIZONTAL),(Truthy(v),VERTICAL)],OFF)`; options
`OfPresent([(Found(on_off),ON),(Found(h),HORIZONTAL),(Found(v),VERTICAL)])` prefixed OFF;
write = three Steps (on_off, vertical, horizontal) each `when=Found(role)`. **F. Fan
turn_on**: `Step(when=Found(switch),True)`, `Step(when=And(Found(speed),ArgPresent(percentage)),
speed,Arg(percentage))`, same for preset; set_percentage:
`Choose([(And(Eq(Arg(percentage),0),Found(switch)),[Step(switch,False)])],[Step(speed,...)])`.
**G. Quirk kt_hw50w7qvxluhslkk**: `when=And(IsInt(Status(temp_set)),Ge(Status(temp_set),450))`.
**H. Climate**: 6.4 + `enum_map_filtered/unfiltered/unmapped_options` + `Append(presets? ...)`.

## 14. Open questions (defaults apply if unanswered)
Q2 Residual/unknown dps: default NONE (exact parity); an OPT-IN residual layer can be added
   later outside the parity tests.
Q3 Raise paths P-13/P-14/P-22: default DEVIATE as tagged (SchemaError per device / UNKNOWN).
Q4 Delta timestamps if the bridge has none: default host synthesizes receive time.
Q5 unique_id: default reference format `tuya.{device_id}{key}` with host-side prefix.
