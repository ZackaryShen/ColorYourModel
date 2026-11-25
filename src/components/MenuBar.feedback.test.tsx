import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { afterEach, expect, it, vi } from "vitest";
import { useAppStore } from "../store/appStore";
import { MenuBar } from "./MenuBar";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn(async () => {}) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => {}) }));

afterEach(() => {
  cleanup();
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

it("opens the feedback form with the UI language, theme, and installed version", () => {
  vi.stubEnv("VITE_FEEDBACK_URL", "https://feedback.example.com/");
  vi.stubGlobal("__APP_VERSION__", "0.2.0");
  useAppStore.setState({ language: "zh", theme: "dark", meshData: null });
  render(<MenuBar onImport={vi.fn()} />);

  fireEvent.click(screen.getByRole("button", { name: "帮助" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "反馈问题…" }));

  expect(invoke).toHaveBeenCalledWith("open_feedback_window", {
    url: "https://feedback.example.com/?lang=zh&version=0.2.0&theme=dark",
    title: "提交反馈",
  });
  expect(openUrl).not.toHaveBeenCalled();
});

it("keeps a clearly labeled GitHub feedback link when the form is not configured", () => {
  vi.stubEnv("VITE_FEEDBACK_URL", "");
  useAppStore.setState({ language: "en", meshData: null });
  render(<MenuBar onImport={vi.fn()} />);

  fireEvent.click(screen.getByRole("button", { name: "Help" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "Report on GitHub…" }));

  expect(openUrl).toHaveBeenCalledWith("https://github.com/ZackaryShen/ColorYourModel/issues/new/choose");
});

it("opens the official form from the always-visible button without a model loaded", () => {
  vi.stubEnv("VITE_FEEDBACK_URL", undefined);
  vi.stubGlobal("__APP_VERSION__", "0.2.0");
  useAppStore.setState({ language: "en", theme: "light", meshData: null });
  render(<MenuBar onImport={vi.fn()} />);

  fireEvent.click(screen.getByRole("button", { name: "Feedback", exact: true }));

  expect(invoke).toHaveBeenCalledWith("open_feedback_window", {
    url: "https://coloryourmodel-feedback.zackaryshen.workers.dev/?lang=en&version=0.2.0&theme=light",
    title: "Send feedback",
  });
  expect(openUrl).not.toHaveBeenCalled();
});

it("shows when the app could not open its feedback window", async () => {
  vi.stubEnv("VITE_FEEDBACK_URL", "https://feedback.example.com/");
  vi.stubGlobal("__APP_VERSION__", "0.2.0");
  vi.mocked(invoke).mockRejectedValueOnce(new Error("Feedback window unavailable"));
  useAppStore.setState({ language: "zh", statusMessage: "status.ready" });
  render(<MenuBar onImport={vi.fn()} />);

  fireEvent.click(screen.getByRole("button", { name: "反馈", exact: true }));

  await vi.waitFor(() => expect(useAppStore.getState().statusMessage).toMatch(/无法打开反馈窗口/));
});
