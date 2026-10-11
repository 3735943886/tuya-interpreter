"""L1 generator: HA core `tuya` description tables -> JSON (pinned to a core checkout).

Usage: python gen_core_tables.py <core/homeassistant/components/tuya> <out dir>
       python build_rules.py --tables <out dir>        (folds them into rules/00-default.json)
Evaluates module-level `NAME: dict[DeviceCategory, tuple[Desc, ...]] = {...}` assignments
symbolically: DPCode.X / DeviceCategory.X -> their string values; other attribute refs
(SwitchDeviceClass.OUTLET, EntityCategory.CONFIG...) -> "$Class.ATTR"; lambdas/calls that
cannot be resolved -> {"$expr": source}. No code is executed. Hand edits are forbidden.
"""
import ast
import json
import pathlib
import sys


def enum_values(tree, cls):
    out={}
    for n in tree.body:
        if isinstance(n,ast.ClassDef) and n.name==cls:
            for s in n.body:
                if isinstance(s,ast.Assign) and isinstance(s.value,ast.Constant):
                    out[s.targets[0].id]=s.value.value
    return out

class Ev:
    def __init__(s, consts, dpcodes, cats):
        s.g=dict(consts); s.dp=dpcodes; s.cat=cats; s.funcs={}; s.imports={}
    def sym(s,c,a):
        """Resolve an enum member to its value via the installed homeassistant (pinned by the run env)."""
        mod=s.imports.get(c)
        if mod:
            try:
                import enum
                import importlib
                v=getattr(getattr(importlib.import_module(mod),c),a)
                return v.value if isinstance(v,enum.Enum) else v
            except Exception: pass  # noqa: BLE001, S110  (best effort: an HA expression that will not evaluate stays symbolic)
        return f'${c}.{a}'
    def __call__(s,n):
        if isinstance(n,ast.Constant): return n.value
        if isinstance(n,ast.Attribute) and isinstance(n.value,ast.Name):
            c,a=n.value.id,n.attr
            if c=='DPCode': return s.dp[a]
            if c=='DeviceCategory': return s.cat[a]
            return s.sym(c,a)
        if isinstance(n,ast.Name):
            if n.id in s.g: return s.g[n.id]
            if n.id in s.imports:
                try:
                    import enum
                    import importlib
                    v=getattr(importlib.import_module(s.imports[n.id]),n.id)
                    if isinstance(v,(str,int,float)): return v.value if isinstance(v,enum.Enum) else v
                except Exception: pass  # noqa: BLE001, S110  (best effort: an HA expression that will not evaluate stays symbolic)
            return f'${n.id}'
        if isinstance(n,(ast.Tuple,ast.List)):
            out=[]
            for e in n.elts:
                if isinstance(e,ast.Starred): out.extend(s(e.value))
                else: out.append(s(e))
            return out
        if isinstance(n,ast.BinOp) and isinstance(n.op,ast.Add):
            a,b=s(n.left),s(n.right)
            if isinstance(a,list) and isinstance(b,list): return a+b
        if isinstance(n,ast.Set): return {'$set':sorted((s(e) for e in n.elts),key=str)}
        if isinstance(n,ast.JoinedStr):
            return ''.join(v.value if isinstance(v,ast.Constant) else str(s(v.value)) for v in n.values)
        if isinstance(n,ast.Dict): return {(s(k) if k is not None else '**'):s(v) for k,v in zip(n.keys,n.values)}
        if isinstance(n,ast.UnaryOp) and isinstance(n.op,ast.USub): return -s(n.operand)
        if isinstance(n,ast.Call) and isinstance(n.func,ast.Name) and n.func.id in s.funcs:
            fn=s.funcs[n.func.id]; saved=dict(s.g)
            for a,v in zip(fn.args.args,n.args): s.g[a.arg]=s(v)
            try:
                ret=[x for x in fn.body if isinstance(x,ast.Return)][-1]
                return s(ret.value)
            finally: s.g=saved
        if isinstance(n,ast.Call):
            f=ast.unparse(n.func)
            if 'Description' in f or f.endswith('description'):
                d={'$desc':f}
                for i,a in enumerate(n.args): d[f'$arg{i}']=s(a)
                for k in n.keywords: d[k.arg if k.arg else '**']=s(k.value)
                return d
            if f=='UnitOfMeasurement':
                return {**{f'a{i}':s(a) for i,a in enumerate(n.args)}, **{k.arg:s(k.value) for k in n.keywords}}
            if f=='dict' or f=='tuple' or f=='list': return [s(a) for a in n.args] if n.args else []
        return {'$expr':ast.unparse(n)}

def gen(core, out):
    consts=ast.parse((core/'const.py').read_text())
    dp=enum_values(consts,'DPCode'); cat=enum_values(consts,'DeviceCategory')
    out.mkdir(parents=True,exist_ok=True)
    ev=Ev({},dp,cat); ct=ast.parse((core/'const.py').read_text())
    for n in ct.body:
        if isinstance(n,ast.ImportFrom) and n.module and n.module.startswith('homeassistant'):
            for al in n.names: ev.imports[al.asname or al.name]=n.module
    for n in ct.body:
        if isinstance(n,ast.Assign) and isinstance(n.targets[0],ast.Name) and isinstance(n.value,ast.Set): ev.g[n.targets[0].id]=ev(n.value)
    units=None
    for n in ct.body:
        if isinstance(n,ast.Assign) and getattr(n.targets[0],'id','')=='UNITS': units=ev(n.value)
    dcu={}
    for u in units:
        dcs=u['device_classes']['$set'] if isinstance(u['device_classes'],dict) else u['device_classes']
        for dc in dcs:
            m=dcu.setdefault(dc,{}); m[u['unit']]=u['unit']
            for al in ((u.get('aliases') or {}).get('$set',[]) if isinstance(u.get('aliases'),dict) else []): m[al]=u['unit']
    (out/'_units.json').write_text(json.dumps(dcu,indent=1,ensure_ascii=False))
    (out/'_enums.json').write_text(json.dumps({'DPCode':dp,'DeviceCategory':cat},indent=1))
    summary={}
    for f in sorted(core.glob('*.py')):
        if f.stem in ('const','__init__','entity','coordinator','config_flow','diagnostics','util','services','scene'): continue
        tree=ast.parse(f.read_text()); ev=Ev({},dp,cat); tables={}; aliases=[]
        for n in tree.body:
            if isinstance(n,ast.ImportFrom) and n.module and n.module.startswith('homeassistant'):
                for al in n.names: ev.imports[al.asname or al.name]=n.module
        for n in tree.body:
            if isinstance(n,ast.FunctionDef): ev.funcs[n.name]=n
            tgt=val=None
            if isinstance(n,ast.Assign) and len(n.targets)==1: tgt,val=n.targets[0],n.value
            elif isinstance(n,ast.AnnAssign) and n.value is not None: tgt,val=n.target,n.value
            if tgt is None: continue
            if isinstance(tgt,ast.Name) and isinstance(val,(ast.Dict,ast.Tuple,ast.List,ast.BinOp,ast.Call)):
                try: v=ev(val)
                except Exception: continue  # noqa: BLE001, S112  (best effort: a table that will not evaluate is skipped)
                ev.g[tgt.id]=v
                if isinstance(val,ast.Dict): tables[tgt.id]=v
            elif isinstance(tgt,ast.Subscript) and isinstance(tgt.value,ast.Name) and isinstance(val,ast.Subscript):
                # ALIAS: TABLE[cat_a] = TABLE[cat_b]  (per-platform alias, spec I8)
                t=tgt.value.id; a=ev(tgt.slice); b=ev(val.slice)
                if t in tables and isinstance(val.value,ast.Name) and val.value.id==t:
                    tables[t][a]=tables[t][b]; aliases.append([t,a,b])
        if tables:
            (out/f'{f.stem}.json').write_text(json.dumps({'tables':tables,'aliases':aliases},indent=1,ensure_ascii=False,default=str))
            summary[f.stem]={t:len(v) for t,v in tables.items()}
    return summary
if __name__=='__main__':
    print(json.dumps(gen(pathlib.Path(sys.argv[1]),pathlib.Path(sys.argv[2])),indent=1))
