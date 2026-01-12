import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import Setup from "./pages/Setup";
import Dashboard from "./pages/Dashboard";
import "./App.css";

function App() {
  const [loading, setLoading] = useState(true);
  const [setupComplete, setSetupComplete] = useState(false);

  useEffect(() => {
    checkConfig();
  }, []);

  async function checkConfig() {
    try {
      const config: any = await invoke("get_config");
      if (config.setup_completed) {
        const hasAuth = await invoke<boolean>("check_auth");
        if (hasAuth) {
          await invoke("start_sync").catch(() => { }); // Try start sync silently
          setSetupComplete(true);
        } else {
          console.log("Config complete but auth missing. Redirecting to setup.");
          setSetupComplete(false); // Force re-login
        }
      } else {
        setSetupComplete(false);
      }
    } catch (error) {
      console.error("Failed to check config:", error);
    } finally {
      setLoading(false);
    }
  }

  if (loading) {
    return <div className="min-h-screen bg-zinc-950 flex items-center justify-center text-zinc-500 font-mono">Loading Xynoxa...</div>;
  }

  if (!setupComplete) {
    return <Setup onComplete={() => setSetupComplete(true)} />;
  }

  return (
    <Dashboard onLogout={() => {
      // Reset setup? Or just logout? Requirement says "Configurable...". 
      // For disconnect, we might want to clear config.
      invoke("logout");
      invoke("save_config", { completed: false }).then(() => setSetupComplete(false));
    }} />
  );
}

export default App;
