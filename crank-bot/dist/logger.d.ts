export type LogLevel = "info" | "warn" | "error" | "debug";
export declare function makeLogger(module: string): {
    info: (msg: string, meta?: Record<string, unknown>) => void;
    warn: (msg: string, meta?: Record<string, unknown>) => void;
    error: (msg: string, meta?: Record<string, unknown>) => void;
    debug: (msg: string, meta?: Record<string, unknown>) => void;
};
//# sourceMappingURL=logger.d.ts.map