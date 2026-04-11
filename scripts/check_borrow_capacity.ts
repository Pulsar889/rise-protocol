import * as anchor from "@coral-xyz/anchor";
import { Connection, PublicKey } from "@solana/web3.js";
import * as fs from "fs";
import * as path from "path";

const RPC = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";

async function main() {
  const conn = new Connection(RPC, "confirmed");
  const prov = new anchor.AnchorProvider(conn, {
    publicKey: PublicKey.default,
    signTransaction: async (t: any) => t,
    signAllTransactions: async (t: any) => t,
  } as any, {});

  const cdpIdl     = JSON.parse(fs.readFileSync(path.join(__dirname, "../target/idl/rise_cdp.json"), "utf-8"));
  const stakingIdl = JSON.parse(fs.readFileSync(path.join(__dirname, "../target/idl/rise_staking.json"), "utf-8"));
  const cdp     = new anchor.Program(cdpIdl, prov) as any;
  const staking = new anchor.Program(stakingIdl, prov) as any;

  const [cfg, pool] = await Promise.all([
    cdp.account.cdpConfig.fetch(PublicKey.findProgramAddressSync([Buffer.from("cdp_config")], cdp.programId)[0]),
    staking.account.globalPool.fetch(PublicKey.findProgramAddressSync([Buffer.from("global_pool")], staking.programId)[0]),
  ]);

  const supply   = BigInt(pool.stakingRiseSolSupply.toString());
  const multBps  = BigInt(cfg.debtCeilingMultiplierBps.toString());
  const minted   = BigInt(cfg.cdpRiseSolMinted.toString());
  const ceiling  = supply * multBps / 10_000n;
  const remaining    = ceiling > minted ? ceiling - minted : 0n;
  const singleCap    = ceiling * 500n / 10_000n;
  const protocolMax  = remaining < singleCap ? remaining : singleCap;

  console.log("staking supply:   ", Number(supply)   / 1e9, "riseSOL");
  console.log("debt ceiling bps: ", cfg.debtCeilingMultiplierBps.toString(), `(${Number(multBps) / 100}%)`);
  console.log("ceiling:          ", Number(ceiling)  / 1e9, "riseSOL");
  console.log("minted so far:    ", Number(minted)   / 1e9, "riseSOL");
  console.log("remaining:        ", Number(remaining)/ 1e9, "riseSOL");
  console.log("single loan cap:  ", Number(singleCap)/ 1e9, "riseSOL  (5% of ceiling)");
  console.log("─────────────────────────────────────────────");
  console.log("protocol max:     ", Number(protocolMax) / 1e9, "riseSOL");
}

main().catch(console.error);
