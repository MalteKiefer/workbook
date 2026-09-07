import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Customer {
  id: number;
  name: string;
  short_code: string;
}

export default function App() {
  const [customers, setCustomers] = useState<Customer[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false })
      .then(setCustomers)
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <main style={{ fontFamily: "sans-serif", padding: "2rem" }}>
      <h1>Wartungsdoku</h1>
      <p>Backend-Kommandos verdrahtet. Command Palette und Editor folgen in späteren Phasen.</p>
      {error && <p style={{ color: "crimson" }}>Fehler: {error}</p>}
      {customers && <p>Kunden in der Datenbank: {customers.length}</p>}
    </main>
  );
}
