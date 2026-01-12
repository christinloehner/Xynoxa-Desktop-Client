import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { homeDir, join } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { enable as enableAutostart } from "@tauri-apps/plugin-autostart";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";

interface SetupProps {
    onComplete: () => void;
}

export default function Setup({ onComplete }: SetupProps) {
    const [step, setStep] = useState(1);
    const [serverUrl, setServerUrl] = useState("https://dev.xynoxa.com");
    const [token, setToken] = useState("");
    const [syncPath, setSyncPath] = useState("");
    const [loading, setLoading] = useState(false);
    const [selectingFolder, setSelectingFolder] = useState(false);
    const [error, setError] = useState("");

    useEffect(() => {
        let active = true;
        const setDefaultPath = async () => {
            try {
                const home = await homeDir();
                if (!home) return;
                const suggested = await join(home, "Xynoxa");
                if (active && !syncPath) {
                    setSyncPath(suggested);
                }
            } catch {
                // Keep empty if we can't resolve a default
            }
        };
        setDefaultPath();
        return () => {
            active = false;
        };
    }, [syncPath]);

    const handleChooseFolder = async () => {
        setError("");
        setSelectingFolder(true);
        try {
            const selected = await open({
                directory: true,
                multiple: false,
                title: "Select Sync Folder"
            });
            if (typeof selected === "string" && selected.length > 0) {
                setSyncPath(selected);
            }
        } catch (e) {
            setError("Folder selection failed: " + e);
        } finally {
            setSelectingFolder(false);
        }
    };

    const handleNext = async () => {
        setError("");

        if (step === 1) {
            // Validate URL format
            try {
                new URL(serverUrl);
                setStep(2);
            } catch {
                setError("Invalid URL");
            }
        } else if (step === 2) {
            // Validate Token (attempt login)
            if (!(token.startsWith("xyn-") || token.startsWith("syn-"))) {
                setError("Token must start with 'xyn-'");
                return;
            }
            setLoading(true);
            try {
                await invoke("login", { token });
                setStep(3);
            } catch (e) {
                setError("Login failed: " + e);
            } finally {
                setLoading(false);
            }
        } else if (step === 3) {
            // Save Config and Finish
            if (!syncPath) {
                setError("Please choose a local sync folder.");
                return;
            }
            setLoading(true);
            try {
                await invoke("save_config", {
                    url: serverUrl,
                    path: syncPath,
                    completed: true
                });
                try {
                    await enableAutostart();
                } catch (e) {
                    console.warn("Failed to enable autostart", e);
                }
                // Start initial sync
                await invoke("start_sync", { token });
                onComplete();
            } catch (e) {
                setError("Setup failed: " + e);
            } finally {
                setLoading(false);
            }
        }
    };

    return (
        <div className="min-h-screen flex items-center justify-center bg-zinc-950 text-white font-sans">
            <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top,_var(--tw-gradient-stops))] from-cyan-900/20 via-zinc-950 to-zinc-950" />
            <Card className="w-full max-w-md bg-zinc-900/50 border-zinc-800 backdrop-blur-xl relative z-10 shadow-2xl">
                <CardHeader>
                    <CardTitle className="text-2xl text-center bg-gradient-to-r from-cyan-300 to-cyan-500 bg-clip-text text-transparent">
                        Setup Xynoxa
                    </CardTitle>
                    <CardDescription className="text-center text-zinc-400">
                        Step {step} of 3
                    </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                    {step === 1 && (
                        <div className="space-y-2">
                            <Label>Server Address</Label>
                            <Input
                                value={serverUrl}
                                onChange={(e) => setServerUrl(e.target.value)}
                                placeholder="https://dev.xynoxa.com"
                                className="bg-zinc-950/50 border-zinc-800"
                            />
                            <p className="text-xs text-zinc-500">Enter the URL of your Xynoxa instance.</p>
                        </div>
                    )}
                    {step === 2 && (
                        <div className="space-y-2">
                            <Label>Personal Access Token</Label>
                            <Input
                                type="password"
                                value={token}
                                onChange={(e) => setToken(e.target.value)}
                                placeholder="xyn-..."
                                className="bg-zinc-950/50 border-zinc-800 font-mono"
                            />
                            <p className="text-xs text-zinc-500">Create a token in your user settings.</p>
                        </div>
                    )}
                    {step === 3 && (
                        <div className="space-y-2">
                            <Label>Local Sync Folder</Label>
                            <div className="flex gap-2">
                                <Input
                                    value={syncPath}
                                    readOnly
                                    placeholder="Select a folder"
                                    className="bg-zinc-950/50 border-zinc-800"
                                />
                                <Button
                                    type="button"
                                    variant="secondary"
                                    onClick={handleChooseFolder}
                                    disabled={loading || selectingFolder}
                                    className="shrink-0"
                                >
                                    {selectingFolder ? "Selecting..." : "Choose"}
                                </Button>
                            </div>
                            <p className="text-xs text-zinc-500">
                                Pick a local folder to sync. You can create a new folder in the dialog.
                            </p>
                        </div>
                    )}
                    {error && <div className="text-sm text-red-400 font-medium">{error}</div>}
                </CardContent>
                <CardFooter className="flex justify-between">
                    {step > 1 && (
                        <Button variant="ghost" onClick={() => setStep(step - 1)} disabled={loading} className="text-zinc-400 hover:text-white">
                            Back
                        </Button>
                    )}
                    <Button
                        onClick={handleNext}
                        disabled={loading}
                        className={`ml-auto bg-cyan-500 hover:bg-cyan-600 text-white ${step === 1 ? 'w-full' : ''}`}
                    >
                        {loading ? "Processing..." : step === 3 ? "Finish Setup" : "Next"}
                    </Button>
                </CardFooter>
            </Card>
        </div>
    );
}
