outcome read : forall 'n, 'n >= 4. int('n) -> int = {
    val helper : int('n) -> int
    impl emulator(x) = helper(x)
}
