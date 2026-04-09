"use strict";
/**
 * Rise Protocol Crank Bot
 *
 * Runs four independent loops:
 *   - Epoch cranks       (update_exchange_rate, collect_fees, collect_cdp_fees, checkpoint_gauges)
 *   - Reward cranks      (checkpoint_borrow_rewards, checkpoint_stake_rewards)
 *   - Liquidation monitor (accrue_interest + liquidate for unhealthy positions)
 *   - Governance monitor  (execute_proposal for passed + timelocked proposals)
 *
 * Configuration via environment variables — see .env.example.
 */
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
const dotenv = __importStar(require("dotenv"));
dotenv.config();
const client_1 = require("./client");
const epochCranks_1 = require("./cranks/epochCranks");
const rewardCranks_1 = require("./cranks/rewardCranks");
const liquidator_1 = require("./cranks/liquidator");
const governance_1 = require("./cranks/governance");
const logger_1 = require("./logger");
const log = (0, logger_1.makeLogger)("main");
const EPOCH_CRANK_INTERVAL_MS = Number(process.env.EPOCH_CRANK_INTERVAL_MS ?? 5 * 60 * 1000); // 5 min
const REWARD_CRANK_INTERVAL_MS = Number(process.env.REWARD_CRANK_INTERVAL_MS ?? 10 * 60 * 1000); // 10 min
const LIQUIDATION_POLL_INTERVAL_MS = Number(process.env.LIQUIDATION_POLL_INTERVAL_MS ?? 30 * 1000); // 30 sec
const GOVERNANCE_POLL_INTERVAL_MS = Number(process.env.GOVERNANCE_POLL_INTERVAL_MS ?? 5 * 60 * 1000); // 5 min
// ── Loop runner ───────────────────────────────────────────────────────────────
/**
 * Runs `fn` immediately, then repeats every `intervalMs`.
 * Errors are caught and logged — the loop continues regardless.
 */
async function runLoop(name, intervalMs, fn) {
    log.info(`loop start: ${name}`, { intervalMs });
    while (true) {
        const start = Date.now();
        try {
            await fn();
        }
        catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            log.error(`loop error: ${name}`, { error: msg });
        }
        const elapsed = Date.now() - start;
        const wait = Math.max(0, intervalMs - elapsed);
        await (0, client_1.sleep)(wait);
    }
}
// ── Main ──────────────────────────────────────────────────────────────────────
async function main() {
    log.info("rise crank bot starting", {
        epochCrankIntervalMs: EPOCH_CRANK_INTERVAL_MS,
        rewardCrankIntervalMs: REWARD_CRANK_INTERVAL_MS,
        liquidationPollIntervalMs: LIQUIDATION_POLL_INTERVAL_MS,
        governancePollIntervalMs: GOVERNANCE_POLL_INTERVAL_MS,
    });
    const client = (0, client_1.createClient)();
    log.info("client ready", { wallet: client.wallet.publicKey.toBase58() });
    // Check bot wallet balance
    const balance = await client.connection.getBalance(client.wallet.publicKey);
    log.info("bot wallet balance", { lamports: balance, sol: balance / 1e9 });
    if (balance < 10000000) { // 0.01 SOL
        log.warn("bot wallet balance is very low — transactions may fail", { lamports: balance });
    }
    // Start all loops concurrently — each runs independently and indefinitely
    await Promise.all([
        runLoop("epoch_cranks", EPOCH_CRANK_INTERVAL_MS, () => (0, epochCranks_1.runEpochCranks)(client)),
        runLoop("reward_cranks", REWARD_CRANK_INTERVAL_MS, () => (0, rewardCranks_1.runRewardCranks)(client)),
        runLoop("liquidation_monitor", LIQUIDATION_POLL_INTERVAL_MS, () => (0, liquidator_1.runLiquidationMonitor)(client)),
        runLoop("governance_monitor", GOVERNANCE_POLL_INTERVAL_MS, () => (0, governance_1.runGovernanceMonitor)(client)),
    ]);
}
main().catch(err => {
    console.error(JSON.stringify({ ts: new Date().toISOString(), level: "error", module: "main", msg: "fatal error", error: err instanceof Error ? err.message : String(err) }));
    process.exit(1);
});
//# sourceMappingURL=index.js.map