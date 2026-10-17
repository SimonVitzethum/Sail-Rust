val select : (bool, int, int) -> int
function select(flag, left, right) = let choice = if flag then left else (false : int) in choice
