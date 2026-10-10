/* Fixture: Tricky-Faelle — Klammern in Kommentaren/Strings. */
val TRICKY : forall ('cold : Bool).
  bool('cold) -> unit

function TRICKY cold = {
    /* block { comment } spanning
       two } lines { */
    // line comment with { brace
    let open = "{ not a brace }";
    let tick = '{';
    if cold then {
        open = "} still { string";
    };
    return()
}
