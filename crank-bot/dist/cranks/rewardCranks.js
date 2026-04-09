"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.checkpointBorrowRewards = checkpointBorrowRewards;
exports.checkpointStakeRewards = checkpointStakeRewards;
exports.runRewardCranks = runRewardCranks;
/**
 * Reward accumulator cranks — called frequently (sub-epoch) for precision:
 *   - checkpoint_borrow_rewards  (cdp)
 *   - checkpoint_stake_rewards   (staking)
 */
const client_1 = require("../client");
const logger_1 = require("../logger");
const log = (0, logger_1.makeLogger)("rewards");
// ── checkpoint_borrow_rewards ─────────────────────────────────────────────────
async function checkpointBorrowRewards(client) {
    const config = await client.cdp.account.borrowRewardsConfig.fetch(client_1.PDAS.borrowRewardsConfig);
    const slot = await client.connection.getSlot("confirmed");
    const lastSlot = Number(config.lastCheckpointSlot.toString());
    if (slot <= lastSlot) {
        log.debug("checkpoint_borrow_rewards: no new slots since last checkpoint");
        return;
    }
    log.info("checkpoint_borrow_rewards: advancing accumulator", { slotDelta: slot - lastSlot });
    await (0, client_1.withRetry)(async () => {
        await client.cdp.methods
            .checkpointBorrowRewards()
            .accounts({
            caller: client.wallet.publicKey,
            borrowRewardsConfig: client_1.PDAS.borrowRewardsConfig,
        })
            .rpc({ commitment: "confirmed" });
    }, "checkpoint_borrow_rewards");
    log.info("checkpoint_borrow_rewards: done");
}
// ── checkpoint_stake_rewards ──────────────────────────────────────────────────
async function checkpointStakeRewards(client) {
    // Only call if the stake_rewards_config account exists (may not be initialized on all deployments)
    const configInfo = await client.connection.getAccountInfo(client_1.PDAS.stakeRewardsConfig, "confirmed");
    if (!configInfo) {
        log.debug("checkpoint_stake_rewards: stake_rewards_config not initialized, skipping");
        return;
    }
    const config = await client.staking.account.stakeRewardsConfig.fetch(client_1.PDAS.stakeRewardsConfig);
    const slot = await client.connection.getSlot("confirmed");
    const lastSlot = Number(config.lastCheckpointSlot.toString());
    if (slot <= lastSlot) {
        log.debug("checkpoint_stake_rewards: no new slots since last checkpoint");
        return;
    }
    log.info("checkpoint_stake_rewards: advancing accumulator", { slotDelta: slot - lastSlot });
    await (0, client_1.withRetry)(async () => {
        await client.staking.methods
            .checkpointStakeRewards()
            .accounts({
            caller: client.wallet.publicKey,
            stakeRewardsConfig: client_1.PDAS.stakeRewardsConfig,
        })
            .rpc({ commitment: "confirmed" });
    }, "checkpoint_stake_rewards");
    log.info("checkpoint_stake_rewards: done");
}
// ── Run both reward cranks ────────────────────────────────────────────────────
async function runRewardCranks(client) {
    log.info("--- reward cranks start ---");
    for (const [name, task] of [
        ["checkpoint_borrow_rewards", () => checkpointBorrowRewards(client)],
        ["checkpoint_stake_rewards", () => checkpointStakeRewards(client)],
    ]) {
        try {
            await task();
        }
        catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            log.error(`${name}: failed after retries`, { error: msg });
        }
    }
    log.info("--- reward cranks done ---");
}
//# sourceMappingURL=rewardCranks.js.map