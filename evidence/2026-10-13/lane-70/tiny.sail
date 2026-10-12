default Order dec

struct Point = { x : int, y : int }

enum Colour = { Red, Green }

union Shape = { Circle : int, Square : bool }

val const_one : unit -> int
function const_one() = 1

val choose : bool -> int
function choose(b) = if b then 1 else 0

val swap_pair : (int, bool) -> (bool, int)
function swap_pair(x, y) = (y, x)

val two_nums : unit -> vector(2, int)
function two_nums() = [1, 2]

val byte_pair : unit -> bitvector(2)
function byte_pair() = [bitone, bitzero]

val is_red : Colour -> bool
function is_red(Red) = true
and is_red(Green) = false

val mk_point : (int, int) -> Point
function mk_point(x, y) = struct Point { x = x, y = y }

val get_x : Point -> int
function get_x(p) = p.x

val describe : Shape -> int
function describe(s) = match s { Circle(n) => n, Square(_) => 0 }
