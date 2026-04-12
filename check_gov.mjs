import { Connection, PublicKey } from "@solana/web3.js";

const conn = new Connection("https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6", "confirmed");
const GOV = new PublicKey("CtMKhgY5xKiwLB5jmQ44PRF9QsUqXqSbiyVbFsidskHz");

const [configPda] = PublicKey.findProgramAddressSync([Buffer.from("governance_config")], GOV);
const [vaultPda]  = PublicKey.findProgramAddressSync([Buffer.from("rise_vault")], GOV);

const [cfg, vault] = await Promise.all([
  conn.getAccountInfo(configPda),
  conn.getAccountInfo(vaultPda),
]);

if (!cfg)   { console.log("governance_config: NOT FOUND"); process.exit(); }
if (!vault) { console.log("rise_vault: NOT FOUND"); process.exit(); }

const riseMintInConfig = new PublicKey(cfg.data.slice(104, 136)).toBase58();
const mintInVault      = new PublicKey(vault.data.slice(0, 32)).toBase58();

console.log("config.rise_mint:", riseMintInConfig);
console.log("vault.mint:      ", mintInVault);
console.log("match:           ", riseMintInConfig === mintInVault);
