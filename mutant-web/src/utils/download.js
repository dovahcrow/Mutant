// mutant-web/src/utils/download.js
window.mutantDownloadWriters = window.mutantDownloadWriters || {};

export async function initSaveFile(fileName) {
    try {
        const handle = await window.showSaveFilePicker({ suggestedName: fileName });
        const writable = await handle.createWritable();
        const writerId = "writer_" + Date.now() + "_" + Math.random().toString(36).substr(2, 9);
        window.mutantDownloadWriters[writerId] = writable.getWriter();
        console.log(`initSaveFile: created writerId ${writerId} for ${fileName}`);
        return { writerId: writerId, error: null };
    } catch (err) {
        console.error("initSaveFile Error:", err);
        return { writerId: null, error: err.message };
    }
}

export async function writeChunk(writerId, uint8ArrayChunk) {
    const writer = window.mutantDownloadWriters[writerId];
    if (!writer) {
        console.error(`writeChunk Error: Writer not found for ID ${writerId}`);
        return { error: "Writer not found" };
    }
    try {
        await writer.write(uint8ArrayChunk);
        return { error: null };
    } catch (err) {
        console.error(`writeChunk Error for ID ${writerId}:`, err);
        return { error: err.message };
    }
}

export async function closeFile(writerId) {
    const writer = window.mutantDownloadWriters[writerId];
    if (!writer) {
        console.error(`closeFile Error: Writer not found for ID ${writerId}`);
        return { error: "Writer not found" };
    }
    try {
        await writer.close();
        delete window.mutantDownloadWriters[writerId];
        console.log(`closeFile: closed and deleted writerId ${writerId}`);
        return { error: null };
    } catch (err) {
        console.error(`closeFile Error for ID ${writerId}:`, err);
        return { error: err.message };
    }
}

export async function abortFile(writerId, reason) {
    const writer = window.mutantDownloadWriters[writerId];
    if (!writer) {
        console.error(`abortFile Error: Writer not found for ID ${writerId}`);
        return { error: "Writer not found" };
    }
    try {
        await writer.abort(reason);
        delete window.mutantDownloadWriters[writerId];
        console.log(`abortFile: aborted and deleted writerId ${writerId} for reason: ${reason}`);
        return { error: null };
    } catch (err) {
        console.error(`abortFile Error for ID ${writerId}:`, err);
        return { error: err.message };
    }
}
