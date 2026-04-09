"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.runGovernanceMonitor = runGovernanceMonitor;
const client_1 = require("../client");
const logger_1 = require("../logger");
const log = (0, logger_1.makeLogger)("governance");
// Proposal discriminator: sha256("account:Proposal")[..8]
// From the IDL discriminator field
const PROPOSAL_DISC = Buffer.from([148, 8, 84, 144, 136, 36, 150, 93]);
async function runGovernanceMonitor(client) {
    log.info("governance monitor: scanning proposals");
    const currentSlot = await client.connection.getSlot("confirmed");
    // Fetch all proposal accounts
    let proposalAccounts;
    try {
        proposalAccounts = await client.connection.getProgramAccounts(client_1.PROGRAM_IDS.governance, {
            commitment: "confirmed",
            filters: [
                { memcmp: { offset: 0, bytes: PROPOSAL_DISC.toString("base64"), encoding: "base64" } },
            ],
        });
    }
    catch (err) {
        log.error("governance monitor: getProgramAccounts failed", { error: err instanceof Error ? err.message : String(err) });
        return;
    }
    log.info("governance monitor: found proposals", { count: proposalAccounts.length });
    let executed = 0;
    for (const { pubkey: proposalPubkey } of proposalAccounts) {
        let proposal;
        try {
            proposal = await client.governance.account.proposal.fetch(proposalPubkey);
        }
        catch {
            continue; // wrong discriminator or corrupt account
        }
        if (proposal.executed) {
            log.debug("governance monitor: proposal already executed", { proposal: proposalPubkey.toBase58() });
            continue;
        }
        const votingEndSlot = Number(proposal.votingEndSlot.toString());
        const executionSlot = Number(proposal.executionSlot.toString());
        if (currentSlot <= votingEndSlot) {
            log.debug("governance monitor: proposal still in voting", {
                proposal: proposalPubkey.toBase58(),
                slotsRemaining: votingEndSlot - currentSlot,
            });
            continue;
        }
        if (currentSlot < executionSlot) {
            log.debug("governance monitor: proposal in timelock", {
                proposal: proposalPubkey.toBase58(),
                slotsRemaining: executionSlot - currentSlot,
            });
            continue;
        }
        // Voting ended and timelock elapsed — attempt execution
        // The program will reject if proposal didn't pass quorum/threshold
        log.info("governance monitor: attempting execute_proposal", {
            proposal: proposalPubkey.toBase58(),
            votesFor: proposal.votesFor.toString(),
            votesAgainst: proposal.votesAgainst.toString(),
            currentSlot,
            executionSlot,
        });
        try {
            await (0, client_1.withRetry)(async () => {
                await client.governance.methods
                    .executeProposal()
                    .accounts({
                    caller: client.wallet.publicKey,
                    config: client_1.PDAS.governanceConfig,
                    proposal: proposalPubkey,
                })
                    .rpc({ commitment: "confirmed" });
            }, `execute_proposal(${proposalPubkey.toBase58().slice(0, 8)})`);
            log.info("governance monitor: proposal executed", { proposal: proposalPubkey.toBase58() });
            executed++;
        }
        catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            // ProposalDidNotPass is expected for failed proposals — log as info not error
            if (msg.includes("ProposalDidNotPass")) {
                log.info("governance monitor: proposal did not pass, skipping", { proposal: proposalPubkey.toBase58() });
            }
            else {
                log.error("governance monitor: execute_proposal failed", { proposal: proposalPubkey.toBase58(), error: msg });
            }
        }
    }
    log.info("governance monitor: scan complete", { executed });
}
//# sourceMappingURL=governance.js.map