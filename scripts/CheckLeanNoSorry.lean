import Lean

-- Inspect elaborated declarations, including opaque theorem bodies and
-- transitive imports. This catches holes introduced by tactics/macros even
-- when no literal `sorry` occurs in generated source or warnings are disabled.
def main (args : List String) : IO UInt32 := do
  let [manifest] := args | throw <| IO.userError "expected module manifest"
  let modules := (← IO.FS.readFile manifest).splitOn "\n" |>.filter (· != "")
  if modules.isEmpty then throw <| IO.userError "no compiled Lean modules to inspect"
  Lean.initSearchPath (← Lean.findSysroot)
  let imports := modules.toArray.map fun moduleName =>
    ({ module := moduleName.splitOn "." |>.foldl Lean.Name.str Lean.Name.anonymous } : Lean.Import)
  let env ← Lean.importModules imports {}
  let holes := env.constants.fold (init := #[]) fun holes name info =>
    if info.type.hasSorry || (info.value? (allowOpaque := true)).any Lean.Expr.hasSorry then
      holes.push name
    else holes
  for name in holes do IO.eprintln s!"Lean proof hole: {name}"
  if !holes.isEmpty then return 1
  IO.println s!"Lean no-sorry check passed for {modules.length} modules and their imports."
  return 0
