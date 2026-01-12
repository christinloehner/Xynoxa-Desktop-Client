import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";

export default function Login({ onLogin }: { onLogin: () => void }) {
    const [token, setToken] = useState("");
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState("");

    const handleLogin = async (e: React.FormEvent) => {
        e.preventDefault();
        setLoading(true);
        setError("");

        try {
            await invoke("login", { token });
            onLogin(); // Navigate to dashboard
        } catch (err) {
            console.error(err);
            setError(typeof err === 'string' ? err : "Invalid token or connection failed");
        } finally {
            setLoading(false);
        }
    };

    return (
        <div className="min-h-screen flex items-center justify-center bg-zinc-950 text-white selection:bg-cyan-500/30 font-sans">
            <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top,_var(--tw-gradient-stops))] from-cyan-900/20 via-zinc-950 to-zinc-950" />
            <Card className="w-full max-w-md bg-zinc-900/50 border-zinc-800 backdrop-blur-xl relative z-10 shadow-2xl shadow-cyan-500/10 animate-in fade-in zoom-in-95 duration-500">
                <CardHeader className="space-y-1 text-center">
                    <CardTitle className="text-3xl font-bold tracking-tight bg-gradient-to-r from-cyan-300 to-cyan-500 bg-clip-text text-transparent">Xynoxa</CardTitle>
                    <CardDescription className="text-zinc-400">
                        Connect your computer to your Xynoxa Cloud
                    </CardDescription>
                </CardHeader>
                <form onSubmit={handleLogin}>
                    <CardContent className="space-y-4">
                        <div className="space-y-2 text-left">
                            <Label htmlFor="token" className="text-zinc-300">Personal Access Token</Label>
                            <Input
                                id="token"
                                type="password"
                                placeholder="xyn-..."
                                value={token}
                                onChange={(e) => setToken(e.target.value)}
                                className="bg-zinc-950/50 border-zinc-800 focus:ring-cyan-500/50 focus:border-cyan-500 text-zinc-100 placeholder:text-zinc-600 transition-all font-mono"
                                required
                            />
                        </div>
                        {error && <div className="text-sm text-red-400 font-medium">{error}</div>}
                    </CardContent>
                    <CardFooter>
                        <Button
                            type="submit"
                            className="w-full bg-cyan-500 hover:bg-cyan-600 text-white shadow-lg shadow-cyan-500/20 transition-all duration-300 h-10 font-medium"
                            disabled={loading}
                        >
                            {loading ? "Connecting..." : "Connect Account"}
                        </Button>
                    </CardFooter>
                </form>
            </Card>
        </div>
    );
}
