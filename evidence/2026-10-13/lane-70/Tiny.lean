-- Lane-70 Tiny-Modell: Signaturen generiert (sail-lean),
-- Bodies handuebersetzt (kein exps.rs am Base-Stand; Owner lane-14).
-- `inductive Colour` manuell (keine Typedef-Emission; Owner lane-30).

inductive Colour where
  | Red
  | Green

-- MANUAL BODY (const_one): 1
def const_one : Int := 1
-- MANUAL BODY (choose): if b then 1 else 0
def choose (b : Bool) : Int := if b then 1 else 0
-- MANUAL BODY (swap_pair): (y, x)
def swap_pair (x : Int) (y : Bool) : (Bool × Int) := (y, x)
-- MANUAL BODY (two_nums): ⟨#[1, 2], rfl⟩
def two_nums : (Vector Int 2) := ⟨#[1, 2], rfl⟩
-- MANUAL BODY (byte_pair): 0b10
def byte_pair : (BitVec 2) := 0b10
-- MANUAL BODY (is_red): match c with | .Red => true | .Green => false
def is_red (c : Colour) : Bool := match c with | .Red => true | .Green => false
