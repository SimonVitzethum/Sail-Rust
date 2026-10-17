default Order dec
type bits('n) = bitvector('n)
struct Request('n : Int, 'pa : Type), 'n > 0 = { pa : 'pa, size : int('n) }
val check_request : forall 'n ('pa : Type), 'n > 0. Request('n, 'pa) -> bool
val mono : forall 'n, 'n >= 0. bits('n) -> bits('n)
val bitvector_length : forall 'n, 'n >= 0. bits('n) -> int('n)
overload length = {bitvector_length}
val equal : forall 'n 'm. (int('n), int('m)) -> bool('n == 'm)
overload operator == = {equal}
val read_tag : forall 'l, 'l in {32, 64}. (int('l), bits('l)) -> bool
outcome read : forall 'n, 'n > 0. Request('n, 'pa) -> unit with 'pa : Type = { val pa_bits : 'pa -> { 'l, 'l in {16, 64}. bits('l) } impl emulator(request) = { let pa = pa_bits(request.pa); let pa = if check_request(request) then mono(pa) else pa; let tag : bool = if length(pa) == 32 then read_tag(32, pa) else read_tag(64, pa); () } }
