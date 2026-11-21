import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

const target = document.getElementById("app");

if (!target) {
  throw new Error("Application mount point #app is missing from index.html");
}

const app = mount(App, { target });

export default app;
