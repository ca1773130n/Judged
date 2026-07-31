//! `judged` — the command-line entry point.
//!
//! Two commands are planned, in this order, because §0.5 says ship the ratchet
//! before the reaper: `judged ratchet` (§9.14) baselines a repository and fails
//! CI only on new findings, and `judged e2` (§10) runs the mutation-injection
//! suite that gates whether an auto-act tier may exist at all.
//!
//! There is deliberately no `judged clean`.

fn main() {
    todo!("CLI: wire `judged ratchet` (§9.14) and `judged e2` (§10)")
}
