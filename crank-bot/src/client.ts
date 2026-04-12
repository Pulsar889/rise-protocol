import * as anchor from "@coral-xyz/anchor";
import { AnchorProvider, Program, Wallet } from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import * as fs from "fs";
import * as path from "path";

// ── Program IDs ───────────────────────────────────────────────────────────────

export const PROGRAM_IDS = {
  staking:    new PublicKey("BnQc6jJMT6mt3mvWuQFAd9vf2T2wWkAYD2uGjCXud6Lo"),
  cdp:        new PublicKey("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR"),
  governance: new PublicKey("CtMKhgY5xKiwLB5jmQ44PRF9QsUqXqSbiyVbFsidskHz"),
  rewards:    new PublicKey("8d3UidB3Ent4493deoozPYDC48XG2SRj7EdD7xW67uj8"),
};

// ── PDAs ──────────────────────────────────────────────────────────────────────

export const PDAS = {
  globalPool:          PublicKey.findProgramAddressSync([Buffer.from("global_pool")],          PROGRAM_IDS.staking)[0],
  poolVault:           PublicKey.findProgramAddressSync([Buffer.from("pool_vault")],            PROGRAM_IDS.staking)[0],
  treasury:            PublicKey.findProgramAddressSync([Buffer.from("protocol_treasury")],     PROGRAM_IDS.staking)[0],
  reserveVault:        PublicKey.findProgramAddressSync([Buffer.from("reserve_vault")],         PROGRAM_IDS.staking)[0],
  veriseVault:         PublicKey.findProgramAddressSync([Buffer.from("verise_vault")],          PROGRAM_IDS.staking)[0],
  stakeRewardsConfig:  PublicKey.findProgramAddressSync([Buffer.from("stake_rewards_config")],  PROGRAM_IDS.staking)[0],
  cdpConfig:           PublicKey.findProgramAddressSync([Buffer.from("cdp_config")],            PROGRAM_IDS.cdp)[0],
  cdpFeeVault:         PublicKey.findProgramAddressSync([Buffer.from("cdp_fee_vault")],         PROGRAM_IDS.cdp)[0],
  borrowRewardsConfig: PublicKey.findProgramAddressSync([Buffer.from("borrow_rewards_config")], PROGRAM_IDS.cdp)[0],
  // SOL payment config — seeds: ["payment_config", SystemProgram.programId]
  solPaymentConfig:    PublicKey.findProgramAddressSync(
    [Buffer.from("payment_config"), new PublicKey("11111111111111111111111111111111").toBuffer()],
    PROGRAM_IDS.cdp
  )[0],
  governanceConfig:    PublicKey.findProgramAddressSync([Buffer.from("governance_config")],     PROGRAM_IDS.governance)[0],
  rewardsConfig:       PublicKey.findProgramAddressSync([Buffer.from("rewards_config")],        PROGRAM_IDS.rewards)[0],
};

// ── Client setup ──────────────────────────────────────────────────────────────

export interface RiseClient {
  connection: Connection;
  provider:   AnchorProvider;
  wallet:     Wallet;
  staking:    Program;
  cdp:        Program;
  governance: Program;
  rewards:    Program;
}

export function createClient(): RiseClient {
  const rpc = process.env.RPC_ENDPOINT;
  if (!rpc) throw new Error("RPC_ENDPOINT env var is required");

  const keypairPath = process.env.KEYPAIR_PATH ??
    `${process.env.HOME}/.config/solana/id.json`;
  const raw = JSON.parse(fs.readFileSync(keypairPath, "utf-8"));
  const keypair = Keypair.fromSecretKey(Uint8Array.from(raw));
  const wallet = new Wallet(keypair);

  const connection = new Connection(rpc, "confirmed");
  const provider   = new AnchorProvider(connection, wallet, { commitment: "confirmed" });
  anchor.setProvider(provider);

  const idlDir = path.join(__dirname, "../../target/idl");

  function loadProgram(name: string, programId: PublicKey): Program {
    const idl = JSON.parse(fs.readFileSync(path.join(idlDir, `${name}.json`), "utf-8"));
    return new Program(idl, provider);
  }

  return {
    connection,
    provider,
    wallet,
    staking:    loadProgram("rise_staking",    PROGRAM_IDS.staking),
    cdp:        loadProgram("rise_cdp",        PROGRAM_IDS.cdp),
    governance: loadProgram("rise_governance", PROGRAM_IDS.governance),
    rewards:    loadProgram("rise_rewards",    PROGRAM_IDS.rewards),
  };
}

// ── Retry helper ──────────────────────────────────────────────────────────────

export async function withRetry<T>(
  fn: () => Promise<T>,
  label: string,
  maxAttempts = 3,
  baseDelayMs = 2000,
): Promise<T> {
  let attempt = 0;
  while (true) {
    try {
      return await fn();
    } catch (err: unknown) {
      attempt++;
      if (attempt >= maxAttempts) throw err;
      const delay = baseDelayMs * Math.pow(2, attempt - 1);
      const msg = err instanceof Error ? err.message : String(err);
      console.warn(JSON.stringify({ ts: new Date().toISOString(), level: "warn", module: "retry", msg: `${label} failed (attempt ${attempt}/${maxAttempts}), retrying in ${delay}ms`, error: msg }));
      await sleep(delay);
    }
  }
}

export function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
