import { createRoot } from "react-dom/client";
import "bootstrap/dist/css/bootstrap.min.css";
import "./styles.css";
import App from "./App";
import { GraphPage } from "./graph/GraphPage";

const root = document.getElementById("root");
if (!root) throw new Error("Missing application root.");
createRoot(root).render(
  location.pathname === "/graph" ? <GraphPage /> : <App />,
);
