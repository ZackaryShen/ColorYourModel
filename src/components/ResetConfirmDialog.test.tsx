import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ResetConfirmDialog } from "./ResetConfirmDialog";
import { useAppStore } from "../store/appStore";

// Pin UI language: the app default follows the OS locale, but these tests
// assert Chinese strings regardless of the environment locale (same pattern
// as the SeedPanel / SegmentsPanel tests).
beforeEach(() => {
  useAppStore.setState({ language: "zh" });
});

// ── Issue #28: the in-app confirm that replaced window.confirm ─────
// The native confirm is intercepted by tauri-plugin-dialog (async +
// ACL-denied), so the destructive guards must use this dialog. These
// tests pin its contract: the body copy states the stakes, the danger
// button is the ONLY thing that confirms, Esc/outside-click cancel,
// and while the reset is in flight both buttons are dead.
describe("ResetConfirmDialog", () => {
  it("renders title and the irreversible-action warning", () => {
    render(<ResetConfirmDialog running={false} onCancel={() => {}} onConfirm={() => {}} />);
    expect(screen.getByText("清空所有分区")).toBeTruthy();
    expect(screen.getByText(/此操作不可撤销/)).toBeTruthy();
  });

  it("cancel button → onCancel, not onConfirm", async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    render(<ResetConfirmDialog running={false} onCancel={onCancel} onConfirm={onConfirm} />);
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("overlay click → onCancel (the dimmed backdrop is also a cancel)", async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    render(<ResetConfirmDialog running={false} onCancel={onCancel} onConfirm={onConfirm} />);
    // DialogShell: overlay div > div[role=alertdialog]; the backdrop is the
    // dialog panel's parent.
    const overlay = screen.getByRole("alertdialog").parentElement as Element;
    await user.click(overlay);
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("danger button → onConfirm exactly once", async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    render(<ResetConfirmDialog running={false} onCancel={onCancel} onConfirm={onConfirm} />);
    await user.click(screen.getByRole("button", { name: "确认清空" }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("Esc → onCancel (same contract as the other DialogShell dialogs)", async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    render(<ResetConfirmDialog running={false} onCancel={onCancel} onConfirm={onConfirm} />);
    await user.keyboard("{Escape}");
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("running → both buttons disabled (no cancel and no double confirm mid-wipe)", async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    render(<ResetConfirmDialog running={true} onCancel={onCancel} onConfirm={onConfirm} />);
    const confirmBtn = screen.getByRole("button", { name: "清空中…" }) as HTMLButtonElement;
    expect(confirmBtn.disabled).toBe(true);
    const cancelBtn = screen.getByRole("button", { name: "取消" }) as HTMLButtonElement;
    expect(cancelBtn.disabled).toBe(true);
    await user.click(confirmBtn);
    await user.click(cancelBtn);
    expect(onConfirm).not.toHaveBeenCalled();
    expect(onCancel).not.toHaveBeenCalled();
  });
});
