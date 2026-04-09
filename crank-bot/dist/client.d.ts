import * as anchor from "@coral-xyz/anchor";
import { AnchorProvider, Program, Wallet } from "@coral-xyz/anchor";
import { Connection } from "@solana/web3.js";
export declare const PROGRAM_IDS: {
    staking: anchor.web3.PublicKey;
    cdp: anchor.web3.PublicKey;
    governance: anchor.web3.PublicKey;
    rewards: anchor.web3.PublicKey;
};
export declare const PDAS: {
    globalPool: anchor.web3.PublicKey;
    poolVault: anchor.web3.PublicKey;
    treasury: anchor.web3.PublicKey;
    treasuryVault: anchor.web3.PublicKey;
    stakeRewardsConfig: anchor.web3.PublicKey;
    cdpConfig: anchor.web3.PublicKey;
    cdpFeeVault: anchor.web3.PublicKey;
    borrowRewardsConfig: anchor.web3.PublicKey;
    governanceConfig: anchor.web3.PublicKey;
    rewardsConfig: anchor.web3.PublicKey;
};
export interface RiseClient {
    connection: Connection;
    provider: AnchorProvider;
    wallet: Wallet;
    staking: Program;
    cdp: Program;
    governance: Program;
    rewards: Program;
}
export declare function createClient(): RiseClient;
export declare function withRetry<T>(fn: () => Promise<T>, label: string, maxAttempts?: number, baseDelayMs?: number): Promise<T>;
export declare function sleep(ms: number): Promise<void>;
//# sourceMappingURL=client.d.ts.map