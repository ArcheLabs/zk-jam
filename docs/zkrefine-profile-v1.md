# ZkRefine Profile v1

The profile is a deliberately narrow JAM 0.7.2 boundary:

- one WorkPackage and one WorkItem;
- FETCH host-call 1, mode 6 (`import_segments[item_index][0]`);
- one 4104-byte imported and exported segment;
- EXPORT host-call 7;
- OpenVM public statement of four 32-byte commitments (128 bytes total).

The checked-in case, result, exports, and minimal WorkReport reference vector
are immutable release inputs. The proof binds the canonical case, program,
result, exports, and profile statement. The verifier checks the WorkReport
package/result/exports boundary without re-executing the reference client.

This profile intentionally excludes gas proof, historical lookup, inner PVM,
other FETCH modes, arbitrary PVM programs, aggregation, recursive wrapping,
AIR/chips, and cost-model claims.
