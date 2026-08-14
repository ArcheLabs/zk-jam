# M4 security model

M4 proves that a specific generated OpenVM executable executed the bounded
translated program on the committed two-word input and produced the stated
output.

Program binding has two parts: the PVM program is canonically encoded and
committed with a translation-version domain separator, and deterministic
Translation plus deterministic guest emission produces the executable. The
guest reveals that program commitment as a public value; the host verifier
recomputes the commitment and compares it with the proof public value.

Input binding is runtime binding. The generated guest reads two `u32` words,
encodes `ExecutionInputV1` as `u32 word_count || little-endian words`, wraps
that payload in the versioned commitment envelope, and reveals the input
commitment. The verifier recomputes the same canonical commitment and compares
it with the proof public value. Golden vectors include distinct inputs,
zeroes, and `u32::MAX` to prevent host/guest encoding drift.

The semantic statement parser is exact: bytes `0..32` are the program
commitment, `32..64` the input commitment, and `64..96` the output. The OpenVM
envelope is exactly 128 bytes; bytes `96..128` are reserved and must all be
zero. Any other length or non-zero padding is rejected. Execute-only preflight
and proof verification use this parser rather than positional truncation or
optional slices.

The output is also taken from the parsed proof public values and compared with
the independent bounded reference executor. Tampering with proof bytes, program
commitment, input commitment, output, input, or a PVM instruction must fail the
M4 statement verification.

M4 does not prove full JAM Refine, Host Calls, JAM state validity, consensus,
availability, accumulate, or Native AIR.

The M4.0.2 Native-versus-translated publication benchmark is diagnostic and
does not alter this correctness claim. Native means a direct equivalent
OpenVM guest, not a Jambda-native PVM runtime. Its embedded PVM commitment is
only a comparable public-values envelope and does not establish the generated
translation binding proved by M4.
