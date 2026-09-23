import { getMeta } from "@/lib/api";

const { healthz } = getMeta();
healthz().then(console.log);

export function App() {
  return <></>;
}

export default App;
