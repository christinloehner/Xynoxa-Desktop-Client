import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
    const [syncPath, setSyncPath] = useState("~/Xynoxa"); // In a real app, use dialog.open
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState("");

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
            setLoading(true);
            try {
                await invoke("save_config", {
                    url: serverUrl,
                    path: syncPath,
                    completed: true
                });
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
                            <Input
                                value={syncPath}
                                onChange={(e) => setSyncPath(e.target.value)}
                                placeholder="~/Xynoxa"
                                className="bg-zinc-950/50 border-zinc-800"
                            />
                            <p className="text-xs text-zinc-500">Files will be synchronized to this folder.</p>
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
