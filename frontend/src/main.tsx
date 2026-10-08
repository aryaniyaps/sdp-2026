import { createRoot } from "react-dom/client";
import "bootstrap/dist/css/bootstrap.min.css";
import { GraphPage } from "./graph/GraphPage";
import { DemoPage } from "./demo/DemoPage";

const root = document.getElementById("root");
if (!root) throw new Error("Missing application root.");
createRoot(root).render(
  location.pathname === "/graph" ? <GraphPage /> : <DemoPage />,
);
