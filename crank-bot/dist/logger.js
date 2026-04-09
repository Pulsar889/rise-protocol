"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.makeLogger = makeLogger;
function timestamp() {
    return new Date().toISOString();
}
function log(level, module, message, meta) {
    const entry = {
        ts: timestamp(),
        level,
        module,
        msg: message,
        ...meta,
    };
    const line = JSON.stringify(entry);
    if (level === "error") {
        console.error(line);
    }
    else {
        console.log(line);
    }
}
function makeLogger(module) {
    return {
        info: (msg, meta) => log("info", module, msg, meta),
        warn: (msg, meta) => log("warn", module, msg, meta),
        error: (msg, meta) => log("error", module, msg, meta),
        debug: (msg, meta) => log("debug", module, msg, meta),
    };
}
//# sourceMappingURL=logger.js.map