default Order dec

$include <prelude.sail>

$[no_enum_number_conversions]
enum E = A | B | C

register r_A : E
register r_B : E
register r_C : E
register r : nat

/* Effectful early return: SailM body, SailME.throw. */
function foreach_earlyreturneffect(n : nat) -> bool = {
  foreach (i from 0 to n) {
    if i > 5 then return false;
    r = r + 1;
  };
  r > n
}

/* Pure early return: plain Bool body, ExceptM throw. */
function foreach_earlyreturnpure(n : nat) -> bool = {
  var res : nat = 0;
  foreach (i from 0 to n) {
    if i > 5 then return false;
    res = res + i;
  };
  res > n
}

/* Early return mixed with exit(): SailME.throw plus throw Error.Exit. */
function ite_early_return_exit(x : bool) -> E = {
  r_A = r_C;
  let y : E = if x then return r_A else r_B;
  if x then exit();
  r_B
}
