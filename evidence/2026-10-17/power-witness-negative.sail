default Order dec
$include <prelude.sail>
val pow_witness : forall 'n, 'n >= 0. int('n) -> { 'p, 'p == 2 ^ 'n & 'p >= 0. int('p) }
val unsigned_witness : forall 'n. bits('n) -> { 'u, 0 <= 'u & 'u < 2 ^ 'n. int('u) }
val mod_witness : forall 'n 'm, 'n >= 0 & 'm >= 0. (int('n), int('m)) -> { 'r, 0 <= 'r & 'r < 'm. int('r) }
val bounded : forall 'n 'l, 'n >= 0 & 'l > 0. (bits('n), int('l)) -> { 'r, 0 <= 'r & 'r < 2 ^ ('l - 1). int('r) }
function bounded(v, l) = mod_witness(unsigned_witness(v), pow_witness(l))
