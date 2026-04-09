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
exports.PDAS = exports.PROGRAM_IDS = void 0;
exports.createClient = createClient;
exports.withRetry = withRetry;
exports.sleep = sleep;
const anchor = __importStar(require("@coral-xyz/anchor"));
const anchor_1 = require("@coral-xyz/anchor");
const web3_js_1 = require("@solana/web3.js");
const fs = __importStar(require("fs"));
const path = __importStar(require("path"));
// ── Program IDs ───────────────────────────────────────────────────────────────
exports.PROGRAM_IDS = {
    staking: new web3_js_1.PublicKey("BnQc6jJMT6mt3mvWuQFAd9vf2T2wWkAYD2uGjCXud6Lo"),
    cdp: new web3_js_1.PublicKey("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR"),
    governance: new web3_js_1.PublicKey("CtMKhgY5xKiwLB5jmQ44PRF9QsUqXqSbiyVbFsidskHz"),
    rewards: new web3_js_1.PublicKey("8d3UidB3Ent4493deoozPYDC48XG2SRj7EdD7xW67uj8"),
};
// ── PDAs ──────────────────────────────────────────────────────────────────────
exports.PDAS = {
    globalPool: web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("global_pool")], exports.PROGRAM_IDS.staking)[0],
    poolVault: web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("pool_vault")], exports.PROGRAM_IDS.staking)[0],
    treasury: web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("protocol_treasury")], exports.PROGRAM_IDS.staking)[0],
    treasuryVault: web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("treasury_vault")], exports.PROGRAM_IDS.staking)[0],
    stakeRewardsConfig: web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("stake_rewards_config")], exports.PROGRAM_IDS.staking)[0],
    cdpConfig: web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("cdp_config")], exports.PROGRAM_IDS.cdp)[0],
    cdpFeeVault: web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("cdp_fee_vault")], exports.PROGRAM_IDS.cdp)[0],
    borrowRewardsConfig: web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("borrow_rewards_config")], exports.PROGRAM_IDS.cdp)[0],
    governanceConfig: web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("governance_config")], exports.PROGRAM_IDS.governance)[0],
    rewardsConfig: web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("rewards_config")], exports.PROGRAM_IDS.rewards)[0],
};
function createClient() {
    const rpc = process.env.RPC_ENDPOINT;
    if (!rpc)
        throw new Error("RPC_ENDPOINT env var is required");
    const keypairPath = process.env.KEYPAIR_PATH ??
        `${process.env.HOME}/.config/solana/id.json`;
    const raw = JSON.parse(fs.readFileSync(keypairPath, "utf-8"));
    const keypair = web3_js_1.Keypair.fromSecretKey(Uint8Array.from(raw));
    const wallet = new anchor_1.Wallet(keypair);
    const connection = new web3_js_1.Connection(rpc, "confirmed");
    const provider = new anchor_1.AnchorProvider(connection, wallet, { commitment: "confirmed" });
    anchor.setProvider(provider);
    const idlDir = path.join(__dirname, "../../target/idl");
    function loadProgram(name, programId) {
        const idl = JSON.parse(fs.readFileSync(path.join(idlDir, `${name}.json`), "utf-8"));
        return new anchor_1.Program(idl, provider);
    }
    return {
        connection,
        provider,
        wallet,
        staking: loadProgram("rise_staking", exports.PROGRAM_IDS.staking),
        cdp: loadProgram("rise_cdp", exports.PROGRAM_IDS.cdp),
        governance: loadProgram("rise_governance", exports.PROGRAM_IDS.governance),
        rewards: loadProgram("rise_rewards", exports.PROGRAM_IDS.rewards),
    };
}
// ── Retry helper ──────────────────────────────────────────────────────────────
async function withRetry(fn, label, maxAttempts = 3, baseDelayMs = 2000) {
    let attempt = 0;
    while (true) {
        try {
            return await fn();
        }
        catch (err) {
            attempt++;
            if (attempt >= maxAttempts)
                throw err;
            const delay = baseDelayMs * Math.pow(2, attempt - 1);
            const msg = err instanceof Error ? err.message : String(err);
            console.warn(JSON.stringify({ ts: new Date().toISOString(), level: "warn", module: "retry", msg: `${label} failed (attempt ${attempt}/${maxAttempts}), retrying in ${delay}ms`, error: msg }));
            await sleep(delay);
        }
    }
}
function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}
//# sourceMappingURL=client.js.map