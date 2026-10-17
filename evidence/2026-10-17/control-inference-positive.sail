val select : (bool, int, int) -> int
function select(flag, left, right) = {
    let choice = if flag then { left } else { right };
    choice
}
val boolean : bool -> bool
function boolean(flag) = let choice = if flag then true else false in choice
val prefix : int -> int
function prefix(left) = let choice = { (); left } in choice
val choose : unit -> { 'n, 'n >= 0. bitvector('n) }
val witness : bool -> unit
function witness(flag) = let choice = if flag then choose() else choose() in ()
