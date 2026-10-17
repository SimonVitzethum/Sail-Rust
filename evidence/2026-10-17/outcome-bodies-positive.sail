outcome read : int -> int = {
    impl emulator(x) = x
    impl symbolic(x) = 4
}
outcome generic : 'a -> 'a with 'a : Type = {
    val identity : forall 'a. 'a -> 'a
    impl emulator(x) = identity(x)
}
outcome convert : 'a -> int with 'a : Type = {
    val helper : 'a -> int
    impl emulator(x) = helper(x)
}
outcome bounds : forall 'n, 'n >= 4. int('n) -> { 'm, 'm >= 4. int('m) } = {
    impl emulator(x) = x
}
