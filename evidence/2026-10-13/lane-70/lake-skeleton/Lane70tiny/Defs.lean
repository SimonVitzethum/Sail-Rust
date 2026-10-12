import Sail
open PreSail

set_option maxHeartbeats 1_000_000_000
set_option maxRecDepth 1_000_000
set_option linter.unusedVariables false
set_option match.ignoreUnusedAlts true

open Sail
open Sail.ConcurrencyInterfaceV1

namespace Lane70tiny

inductive Register : Type where
  | status
  deriving DecidableEq, Hashable, Repr
open Register

abbrev RegisterType : Register → Type
  | .status => Bool


abbrev exception := Unit

abbrev SailM := PreSailM RegisterType trivialChoiceSource exception
abbrev SailME := PreSailME RegisterType trivialChoiceSource exception


instance : Inhabited (RegisterRef RegisterType Bool) where
  default := .Reg status
