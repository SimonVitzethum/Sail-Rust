// Fixture mirroring the *structure* of
// sail-arm/arm-v9.4-a/src/termination_measures.sail:
// one function measure, single/multiple while measures, repeat bounds.
termination_measure Reduce__1(op,input,esize,altfp) = 'N

termination_measure FloorPow2
  while x - pow2(n)

termination_measure ImplementedSMEVectorLength
  while nbits, while maxbits - nbits

termination_measure AArch64_S2Walk repeat 100, repeat 100

termination_measure enabled(e) = measure(e)

termination_measure enabled2(_, x) = measure2(x)
