/* outer comment /* nested with unbalanced { brace */ still comment } */

val operator ^^ = "replicate_bits" : forall 'n 'm. (bits('n), int('m)) -> bits('n * 'm)

infix 4 >=_s

overload operator << = {shift_bits_left, shift_left}

scattered function step

function clause step (Fetch()) = {
  x = 1;
}

function clause step (Execute()) = done

mapping clause csr_map = 0x14D <-> "stimecmp"

end step
