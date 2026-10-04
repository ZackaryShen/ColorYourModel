import { beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { UpdateDialog } from "./UpdateDialog";
import { useUpdateStore, type UpdateInfo } from "../../store/updateStore";
import { useAppStore } from "../../store/appStore";

/**
 * Render-level coverage for the update popup itself (the store's policy and
 * state-machine guards live in updateStore.test.ts). All phases rendered here
 * make NO Tauri calls: the progress listener is only subscribed while
 * downloading and rejects harmlessly in jsdom (caught + logged).
 *
 * No jest-dom: this repo's setup.ts doesn't register it — getBy* throwing on
 * miss IS the presence assertion; queryBy* + toBeNull covers absence.
 */

const info: UpdateInfo = {
  current: "0.1.1",
  latest: "9.9.9",
  notes: "- fix a\n- add b",
  releaseUrl: "https://github.com/ZackaryShen/ColorYourModel/releases",
  canAutoInstall: true,
  downloadUrl: "https://example.com/ColorYourModel_9.9.9_x64-setup.exe",
  assetSize: 3_590_475,
};

const reset = () => {
  useAppStore.setState({ language: "zh", paintDirty: false, meshData: null });
  useUpdateStore.setState({
    autoCheckEnabled: true,
    skippedVersion: null,
    phase: "idle",
    info: null,
    errorReason: null,
    errorFrom: null,
    progress: { received: 0, total: 0 },
  });
};

beforeEach(reset);

describe("UpdateDialog render phases", () => {
  it("renders nothing while idle", () => {
    const { container } = render(<UpdateDialog />);
    expect(container.innerHTML).toBe("");
  });

  it("available: title, versions, notes and the four decision buttons", () => {
    useUpdateStore.setState({ phase: "available", info });
    render(<UpdateDialog />);
    screen.getByText("发现新版本 v9.9.9");
    screen.getByText(/当前版本 v0\.1\.1/);
    screen.getByText(/fix a/);
    screen.getByRole("button", { name: "立即更新" });
    screen.getByRole("button", { name: "稍后" });
    screen.getByRole("button", { name: "跳过此版本" });
    screen.getByRole("button", { name: "不再提醒" });
  });

  it("available without an installable asset offers the releases page instead", () => {
    useUpdateStore.setState({
      phase: "available",
      info: { ...info, canAutoInstall: false, downloadUrl: null },
    });
    render(<UpdateDialog />);
    screen.getByRole("button", { name: "前往发布页" });
    expect(screen.queryByRole("button", { name: "立即更新" })).toBeNull();
  });

  it("available warns about unexported paint before any download starts", () => {
    useAppStore.setState({ paintDirty: true, meshData: { faceCount: 12 } as never });
    useUpdateStore.setState({ phase: "available", info });
    render(<UpdateDialog />);
    screen.getByText(/未导出的涂装将不会保留/);
  });

  it("downloading: progress bar reflects received/total and MB text", () => {
    useUpdateStore.setState({
      phase: "downloading",
      info,
      progress: { received: 1_048_576, total: 3_590_475 },
    });
    render(<UpdateDialog />);
    const bar = screen.getByRole("progressbar");
    // 1048576 / 3590475 ≈ 29.2%
    expect(bar.getAttribute("aria-valuenow")).toBe("29");
    screen.getByText("已下载 1.0 / 3.4 MB");
    screen.getByRole("button", { name: "取消" });
  });

  it("ready: offers install-and-restart plus cancel", () => {
    useUpdateStore.setState({ phase: "ready", info });
    render(<UpdateDialog />);
    screen.getByRole("button", { name: "立即安装并重启" });
    screen.getByRole("button", { name: "取消" });
  });

  it("installing: locked alertdialog with no buttons", () => {
    useUpdateStore.setState({ phase: "installing", info });
    render(<UpdateDialog />);
    screen.getByRole("alertdialog");
    screen.getByText(/正在退出并安装/);
    expect(screen.queryAllByRole("button")).toHaveLength(0);
  });

  it("error: retry routes by failure source (download → re-download)", () => {
    useUpdateStore.setState({ phase: "error", info, errorFrom: "download", errorReason: "boom" });
    render(<UpdateDialog />);
    screen.getByRole("button", { name: "重试" });
    screen.getByText("boom");
  });

  it("error from a check failure keeps releases-page fallback out", () => {
    useUpdateStore.setState({
      phase: "error",
      info: null,
      errorFrom: "check",
      errorReason: "HTTP 403",
    });
    render(<UpdateDialog />);
    screen.getByText("HTTP 403");
    screen.getByRole("button", { name: "重试" });
    expect(screen.queryByRole("button", { name: "前往发布页" })).toBeNull();
  });
});
