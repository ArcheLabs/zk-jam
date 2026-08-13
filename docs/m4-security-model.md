# M4 security model

M4 proves that a specific generated OpenVM executable executed the bounded
translated program on the committed two-word input and produced the stated
output.

Program binding has two parts: the PVM program is canonically encoded and
committed with a translation-version domain separator, and deterministic
Translation plus deterministic guest emission produces the executable. The
guest reveals that program commitment as a public value; the host verifier
recomputes the commitment and compares it with the proof public value.

Input binding is runtime binding. The generated guest reads `[u32; 2]`, hashes
the canonical input encoding inside the guest, and reveals that input
commitment. The verifier compares it with the expected input commitment.

The output is also taken from the proof public values and compared with the
independent bounded reference executor. Tampering with proof bytes, program
commitment, input commitment, output, input, or a PVM instruction must fail the
M4 statement verification.

M4 does not prove full JAM Refine, Host Calls, JAM state validity, consensus,
availability, accumulate, or Native AIR.
