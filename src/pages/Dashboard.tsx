import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import logo from "@/assets/xynoxa-logo-dark.png";

type SyncState = "idle" | "pulling" | "pushing" | "syncing";

export default function Dashboard({ onLogout }: { onLogout: () => void }) {
    const [syncStatus, setSyncStatus] = useState<SyncState>("idle");
    const [syncPath, setSyncPath] = useState("");

    useEffect(() => {
        loadConfig();
        startSyncOnMount();
    }, []);

    const loadConfig = async () => {
        try {
            const config: any = await invoke("get_config");
            if (config.sync_path) {
                setSyncPath(config.sync_path);
            }
        } catch (e) {
            console.error("Failed to load config", e);
        }
    };

    const startSyncOnMount = async () => {
        try {
            setSyncStatus("syncing");
            await invoke("start_sync");
            setSyncStatus("idle");
        } catch (e) {
            console.error(e);
            setSyncStatus("idle");
        }
    };

    const getStatusDisplay = () => {
        switch (syncStatus) {
            case "pulling":
                return { text: "Downloading...", color: "text-blue-400", dot: "bg-blue-400 animate-pulse" };
            case "pushing":
                return { text: "Uploading...", color: "text-cyan-400", dot: "bg-cyan-400 animate-pulse" };
            case "syncing":
                return { text: "Syncing...", color: "text-amber-400", dot: "bg-amber-400 animate-pulse" };
            default:
                return { text: "All files synced", color: "text-green-400", dot: "bg-green-400" };
        }
    };

    const status = getStatusDisplay();

    return (
        <div className="min-h-screen bg-gradient-to-b from-zinc-950 to-zinc-900 text-zinc-100 flex flex-col items-center px-6 py-8 font-sans">
            {/* Logo */}
            <div className="mb-8">
                <img src={logo} alt="Xynoxa" className="h-10 object-contain" />
            </div>

            {/* Description */}
            <div className="text-center mb-10 max-w-xs">
                <p className="text-zinc-400 text-sm leading-relaxed">
                    Your files, always in sync. Xynoxa keeps your local folder synchronized with the cloud — automatically and securely.
                </p>
            </div>

            {/* Sync Status Card */}
            <div className="w-full max-w-xs bg-zinc-800/50 backdrop-blur-sm rounded-2xl border border-zinc-700/50 p-6 mb-6">
                <div className="text-xs text-zinc-500 uppercase tracking-wider mb-4 font-medium">
                    Sync Status
                </div>

                <div className="flex items-center gap-3 mb-4">
                    <div className={`w-3 h-3 rounded-full ${status.dot}`} />
                    <span className={`text-lg font-medium ${status.color}`}>
                        {status.text}
                    </span>
                </div>

                {syncPath && (
                    <div className="text-xs text-zinc-500 truncate" title={syncPath}>
                        <span className="text-zinc-600">Folder:</span>{" "}
                        <span className="text-zinc-400 font-mono">{syncPath}</span>
                    </div>
                )}
            </div>

            {/* Disconnect Button */}
            <button
                onClick={onLogout}
                className="text-sm text-zinc-500 hover:text-red-400 transition-colors mb-auto"
            >
                Disconnect
            </button>

            {/* Spacer */}
            <div className="flex-1" />

            {/* Copyright */}
            <footer className="text-center text-xs text-zinc-600 mt-8">
                <p>Xynoxa © 2025 Christin Löhner</p>
                <a
                    href="https://www.xynoxa.com"
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-zinc-500 hover:text-cyan-400 transition-colors"
                >
                    www.xynoxa.com
                </a>
            </footer>
        </div>
    );
}
