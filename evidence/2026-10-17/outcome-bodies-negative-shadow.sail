outcome read : 'a -> 'a with 'a : Type = {
    val identity : forall 'a. 'a -> 'a
    impl emulator(x) = identity(false)
}
