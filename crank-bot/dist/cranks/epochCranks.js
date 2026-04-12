"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.updateExchangeRate = updateExchangeRate;
exports.collectFees = collectFees;
exports.collectCdpFees = collectCdpFees;
exports.checkpointAllGauges = checkpointAllGauges;
exports.runEpochCranks = runEpochCranks;
/**
 * Epoch cranks — called once per epoch:
 *   - update_exchange_rate  (staking)
 *   - collect_fees          (staking)
 *   - collect_cdp_fees      (cdp)
 *   - checkpoint_gauge      (rewards, once per active gauge)
 */
const anchor = __importStar(require("@coral-xyz/anchor"));
const web3_js_1 = require("@solana/web3.js");
const client_1 = require("../client");
const logger_1 = require("../logger");
const log = (0, logger_1.makeLogger)("epoch");
// ── update_exchange_rate ──────────────────────────────────────────────────────
async function updateExchangeRate(client) {
    const pool = await client.staking.account.globalPool.fetch(client_1.PDAS.globalPool);
    const clock = await client.connection.getEpochInfo();
    const currentEpoch = BigInt(clock.epoch);
    const lastEpoch = BigInt(pool.lastRateUpdateEpoch.toString());
    if (currentEpoch <= lastEpoch) {
        log.info("update_exchange_rate: already updated this epoch", { currentEpoch: currentEpoch.toString(), lastEpoch: lastEpoch.toString() });
        return;
    }
    log.info("update_exchange_rate: calling crank", { currentEpoch: currentEpoch.toString() });
    await (0, client_1.withRetry)(async () => {
        // stakeLamportsTotal = 0 until validator delegation is built
        await client.staking.methods
            .updateExchangeRate(new anchor.BN(0))
            .accounts({
            caller: client.wallet.publicKey,
            pool: client_1.PDAS.globalPool,
            poolVault: client_1.PDAS.poolVault,
        })
            .rpc({ commitment: "confirmed" });
    }, "update_exchange_rate");
    log.info("update_exchange_rate: done");
}
// ── collect_fees (staking) ────────────────────────────────────────────────────
async function collectFees(client) {
    const treasury = await client.staking.account.protocolTreasury.fetch(client_1.PDAS.treasury);
    const clock = await client.connection.getEpochInfo();
    const currentEpoch = BigInt(clock.epoch);
    const lastEpoch = BigInt(treasury.lastCollectionEpoch.toString());
    if (currentEpoch <= lastEpoch) {
        log.info("collect_fees: already collected this epoch", { currentEpoch: currentEpoch.toString() });
        return;
    }
    const teamWallet = treasury.teamWallet;
    log.info("collect_fees: calling crank", { currentEpoch: currentEpoch.toString(), teamWallet: teamWallet.toBase58() });
    await (0, client_1.withRetry)(async () => {
        await client.staking.methods
            .collectFees()
            .accounts({
            caller: client.wallet.publicKey,
            pool: client_1.PDAS.globalPool,
            treasury: client_1.PDAS.treasury,
            poolVault: client_1.PDAS.poolVault,
            treasuryVault: client_1.PDAS.treasuryVault,
            teamWallet,
            systemProgram: web3_js_1.SystemProgram.programId,
        })
            .rpc({ commitment: "confirmed" });
    }, "collect_fees");
    log.info("collect_fees: done");
}
// ── collect_cdp_fees ──────────────────────────────────────────────────────────
async function collectCdpFees(client) {
    const feeVaultBalance = await client.connection.getBalance(client_1.PDAS.cdpFeeVault);
    const minSweepLamports = 1000000; // 0.001 SOL — not worth sweeping dust
    if (feeVaultBalance < minSweepLamports) {
        log.info("collect_cdp_fees: fee vault balance too low to sweep", { balanceLamports: feeVaultBalance });
        return;
    }
    log.info("collect_cdp_fees: sweeping fees", { balanceLamports: feeVaultBalance });
    await (0, client_1.withRetry)(async () => {
        await client.cdp.methods
            .collectCdpFees()
            .accounts({
            caller: client.wallet.publicKey,
            cdpFeeVault: client_1.PDAS.cdpFeeVault,
            cdpConfig: client_1.PDAS.cdpConfig,
            treasury: client_1.PDAS.treasury,
            treasuryVault: client_1.PDAS.treasuryVault,
            globalPool: client_1.PDAS.globalPool,
            poolVault: client_1.PDAS.poolVault,
            stakingProgram: client_1.PROGRAM_IDS.staking,
            systemProgram: web3_js_1.SystemProgram.programId,
        })
            .rpc({ commitment: "confirmed" });
    }, "collect_cdp_fees");
    log.info("collect_cdp_fees: done");
}
// ── checkpoint_gauge (once per active gauge per epoch) ───────────────────────
async function checkpointAllGauges(client) {
    // Gauge discriminator: sha256("account:Gauge")[..8]
    const GAUGE_DISC = Buffer.from([9, 19, 249, 189, 158, 171, 226, 205]);
    // Fetch all gauge accounts belonging to the rewards program
    const gaugeAccounts = await client.connection.getProgramAccounts(client_1.PROGRAM_IDS.rewards, {
        commitment: "confirmed",
        filters: [
            { memcmp: { offset: 0, bytes: GAUGE_DISC.toString("base64"), encoding: "base64" } },
        ],
    });
    if (gaugeAccounts.length === 0) {
        log.info("checkpoint_gauge: no gauges found");
        return;
    }
    // Fetch rewards config to check current epoch
    const rewardsConfig = await client.rewards.account.rewardsConfig.fetch(client_1.PDAS.rewardsConfig);
    const currentEpoch = BigInt(rewardsConfig.currentEpoch.toString());
    log.info("checkpoint_gauge: checking gauges", { count: gaugeAccounts.length, currentEpoch: currentEpoch.toString() });
    for (const { pubkey: gaugePda } of gaugeAccounts) {
        let gauge;
        try {
            gauge = await client.rewards.account.gauge.fetch(gaugePda);
        }
        catch {
            // Not a gauge account (wrong discriminator) — skip
            continue;
        }
        if (!gauge.active) {
            log.debug("checkpoint_gauge: gauge inactive, skipping", { gauge: gaugePda.toBase58() });
            continue;
        }
        const lastCheckpoint = BigInt(gauge.lastCheckpointEpoch.toString());
        if (lastCheckpoint >= currentEpoch) {
            log.info("checkpoint_gauge: already checkpointed this epoch", { gauge: gaugePda.toBase58() });
            continue;
        }
        log.info("checkpoint_gauge: checkpointing gauge", { gauge: gaugePda.toBase58(), pool: gauge.pool.toBase58() });
        await (0, client_1.withRetry)(async () => {
            await client.rewards.methods
                .checkpointGauge()
                .accounts({
                caller: client.wallet.publicKey,
                config: client_1.PDAS.rewardsConfig,
                gauge: gaugePda,
            })
                .rpc({ commitment: "confirmed" });
        }, `checkpoint_gauge(${gaugePda.toBase58().slice(0, 8)})`);
        log.info("checkpoint_gauge: done", { gauge: gaugePda.toBase58() });
    }
}
// ── Run all epoch cranks ──────────────────────────────────────────────────────
async function runEpochCranks(client) {
    log.info("--- epoch cranks start ---");
    const tasks = [
        ["update_exchange_rate", () => updateExchangeRate(client)],
        ["collect_fees", () => collectFees(client)],
        ["collect_cdp_fees", () => collectCdpFees(client)],
        ["checkpoint_gauges", () => checkpointAllGauges(client)],
    ];
    for (const [name, task] of tasks) {
        try {
            await task();
        }
        catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            log.error(`${name}: failed after retries`, { error: msg });
        }
    }
    log.info("--- epoch cranks done ---");
}
//# sourceMappingURL=epochCranks.js.map