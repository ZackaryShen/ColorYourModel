const copy = {
  en: {
    title: "Send feedback",
    intro: "Report a bug or suggest an improvement. You can submit without a GitHub account.",
    kind: "Feedback type", bug: "Bug report", feature: "Feature request",
    bugHint: "Something is broken or behaves unexpectedly", featureHint: "An idea to make ColorYourModel better",
    subject: "Title", description: "Description", required: "Required",
    bugTitle: "For example, 3MF export loses the painted colors",
    featureTitle: "For example, add a way to reuse a color palette",
    bugPrompt: "What were you doing? What did you expect, and what happened instead?",
    featurePrompt: "What would you like to do? How would this change help?",
    optional: "Additional details (optional)", steps: "Steps to reproduce or extra details",
    stepsPrompt: "For example, import a model, paint a region, then export to 3MF.",
    version: "App version", os: "Operating system", select: "Select a system",
    publicTitle: "Your feedback will be public",
    publicHint: "Your title, description and any details you add will appear in a public GitHub issue. Remove personal information and private project details first.",
    consent: "I agree to publish this feedback as a public issue.",
    submit: "Submit feedback", submitting: "Submitting…", browse: "Browse GitHub issues",
    created: "Issue created", viewIssue: "View GitHub issue", again: "Send more feedback",
    createdHint: "Thanks for helping improve ColorYourModel. Use the issue link to follow the response.",
    invalid: "Check the title, description, and public posting confirmation, then submit again.",
    challenge: "Complete the verification again before submitting.",
    challengePending: "Complete the verification above to send your feedback.",
    unavailable: "The feedback form is temporarily unavailable.",
    verificationUnavailable: "Verification is temporarily unavailable. Try again in a moment.",
    limited: "Too many submissions. Wait a minute and try again.",
    failed: "GitHub did not confirm the submission. Check the issues list before trying again.",
    noscript: "Enable JavaScript to complete the spam check.", other: "Other",
  },
  zh: {
    title: "提交反馈",
    intro: "报告问题或提出建议，无需 GitHub 账号。",
    kind: "反馈类型", bug: "报告问题", feature: "功能建议",
    bugHint: "功能出错或表现不符合预期", featureHint: "分享你希望增加或改进的功能",
    subject: "标题", description: "描述", required: "必填",
    bugTitle: "例如：导出 3MF 后颜色丢失",
    featureTitle: "例如：希望可以复用配色方案",
    bugPrompt: "你当时在做什么？预期会怎样，实际发生了什么？",
    featurePrompt: "你希望做什么？这个改进能帮你解决什么问题？",
    optional: "补充信息（选填）", steps: "复现步骤或补充说明",
    stepsPrompt: "例如：导入模型、给一个分区上色，然后导出 3MF。",
    version: "应用版本", os: "操作系统", select: "请选择系统",
    publicTitle: "反馈内容会公开",
    publicHint: "标题、描述和补充信息会出现在公开的 GitHub Issue 中。请先移除个人信息和保密项目内容。",
    consent: "我同意将本次反馈发布为公开 Issue。",
    submit: "提交反馈", submitting: "正在提交…", browse: "浏览 GitHub Issues",
    created: "Issue 已创建", viewIssue: "查看 GitHub Issue", again: "继续反馈",
    createdHint: "感谢你帮我们改进 ColorYourModel。可以通过 Issue 链接查看后续回复。",
    invalid: "请检查标题、描述和公开发布确认，再提交。",
    challenge: "请重新完成人机验证后再提交。",
    challengePending: "请先完成上方的人机验证，再提交反馈。",
    unavailable: "反馈表单暂时无法使用。",
    verificationUnavailable: "人机验证暂时不可用，请稍后重试。",
    limited: "提交过于频繁，请等待一分钟再试。",
    failed: "GitHub 未确认提交结果，请先查看 Issues 列表，再决定是否重试。",
    noscript: "请启用 JavaScript 完成人机验证。", other: "其他",
  },
};

function escapeHtml(value = "") {
  return String(value).replace(/[&<>"']/g, character => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[character]);
}

export function renderFeedback({ form, status, error, issueNumber, issueList, limits, systems, siteKey }) {
  const lang = form.lang === "zh" ? "zh" : "en";
  const t = copy[lang];
  const nonce = crypto.randomUUID();
  const showForm = !issueNumber && status !== 503;
  const { theme } = form;
  const returnParams = new URLSearchParams({ lang, version: form.version ?? "" });
  if (theme) returnParams.set("theme", theme);
  const returnUrl = `/?${returnParams}`;
  const submitUrl = theme ? `/submit?theme=${theme}` : "/submit";
  const issueListLink = `<a href="${issueList}">${t.browse}</a>`;
  const kind = form.kind === "feature" ? "feature" : "bug";
  const html = `<!doctype html>
<html lang="${lang === "zh" ? "zh-CN" : "en"}"${theme ? ` data-theme="${theme}"` : ""}>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${issueNumber ? t.created : t.title} · ColorYourModel</title>
  ${showForm ? `<script nonce="${nonce}" src="https://challenges.cloudflare.com/turnstile/v0/api.js" defer></script>` : ""}
  <style nonce="${nonce}">
    :root { color-scheme: light dark; --page:#f3f6fa; --panel:#fff; --ink:#172033; --dim:#4b5870; --line:#aab6c5; --link:#1158a6; --button:#164f91; --tint:#eef5ff; --soft:#f6f8fb; --error:#951d25; --error-bg:#fff1f2; }
    @media (prefers-color-scheme: dark) {
      :root { --page:#10151f; --panel:#171f2e; --ink:#edf3fc; --dim:#bac8db; --line:#60718b; --link:#8fc3ff; --button:#1a568f; --tint:#233750; --soft:#1e293b; --error:#ffc1c5; --error-bg:#391d28; }
    }
    /* The desktop palette follows src/theme.css. The standalone page follows the OS. */
    :root[data-theme="light"] {
      color-scheme:light;
      --page:#fff; --panel:#fff; --ink:#1a1a1a; --dim:#555; --line:#ccc;
      --link:#1565c0; --button:#1565c0; --button-ink:#fff;
      --tint:#bcd4ee; --soft:#e8e8e8; --error:#a02830; --error-bg:#fff1f2;
    }
    :root[data-theme="dark"] {
      color-scheme:dark;
      --page:#2d2d2d; --panel:#1e1e1e; --ink:#eee; --dim:#bbb; --line:#555;
      --link:#4a9eff; --button:#4a9eff; --button-ink:#1a1a1a;
      --tint:#3a5a7a; --soft:#3a3a3a; --error:#ffc1c5; --error-bg:#391d28;
    }
    * { box-sizing:border-box; }
    body { margin:0; background:var(--page); color:var(--ink); font:1rem/1.55 system-ui,sans-serif; }
    main { width:min(40rem,calc(100% - 2rem)); margin:clamp(1rem,5vw,3.5rem) auto; padding:clamp(1rem,5vw,2.25rem); background:var(--panel); border:1px solid var(--line); border-radius:1rem; }
    header { margin-bottom:1.35rem; }
    header p { color:var(--dim); margin:0; }
    .brand { color:var(--link); font-size:.875rem; font-weight:650; }
    h1 { margin:.25rem 0 .5rem; line-height:1.25; }
    form { display:grid; gap:1rem; }
    label { display:grid; gap:.3rem; }
    fieldset { min-width:0; margin:0; padding:0; border:0; }
    legend { padding:0; margin-bottom:.35rem; }
    .field-name, legend { font-weight:600; }
    .required { margin-inline-start:.5ch; font-size:.825rem; font-weight:400; color:var(--dim); }
    .metadata,.choices { display:grid; gap:.65rem; grid-template-columns:repeat(auto-fit,minmax(min(100%,12rem),1fr)); }
    .choice { display:flex; align-items:flex-start; gap:.5rem; padding:.65rem; min-height:3.25rem; border:1px solid var(--line); border-radius:.5rem; cursor:pointer; }
    .choice:has(:checked) { border-color:var(--link); background:var(--tint); }
    .choice input, .consent input { flex:none; width:1rem; height:1rem; margin:.22rem 0 0; accent-color:var(--button); }
    .choice small { display:block; color:var(--dim); line-height:1.35; }
    input[type=text],textarea,select { width:100%; min-width:0; border:1px solid var(--line); border-radius:.45rem; padding:.55rem .65rem; background:var(--panel); color:var(--ink); font:inherit; }
    ::placeholder { color:var(--dim); opacity:1; }
    textarea { resize:vertical; }
    details,.publication { background:var(--soft); border-radius:.55rem; padding:.75rem; }
    summary { cursor:pointer; font-weight:600; }
    .summary-version { font-size:.875rem; color:var(--dim); overflow-wrap:anywhere; font-weight:400; }
    .extra-fields { margin-top:.75rem; display:grid; gap:.75rem; }
    .publication p { margin:0 0 .65rem; color:var(--dim); font-size:.9rem; }
    .consent { display:flex; align-items:flex-start; gap:.55rem; color:var(--dim); }
    button,.primary-link { padding:.65rem 1rem; border:0; border-radius:.45rem; background:var(--button); color:var(--button-ink,#fff); font:inherit; font-weight:600; cursor:pointer; }
    .primary-link { display:inline-block; text-decoration:none; }
    button:hover:not(:disabled),.primary-link:hover { filter:brightness(1.12); }
    button:disabled { opacity:.75; cursor:wait; }
    a { color:var(--link); }
    a:focus-visible,button:focus-visible,input:focus-visible,select:focus-visible,textarea:focus-visible,summary:focus-visible { outline:2px solid var(--link); outline-offset:2px; }
    .error { background:var(--error-bg); color:var(--error); padding:.75rem; border-radius:.45rem; }
    .verification-status { margin:0; color:var(--error); }
    .verification-status:not(:empty) { margin-top:.35rem; }
    .actions { display:flex; flex-wrap:wrap; align-items:center; justify-content:space-between; gap:.6rem 1rem; }
    footer { margin-top:1.5rem; }

    :root[data-theme] body { font:13px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif; }
    :root[data-theme] main { width:100%; max-width:660px; margin:0 auto; padding:16px 20px; background:none; border:0; border-radius:0; }
    :root[data-theme] header { margin-bottom:14px; }
    :root[data-theme] h1 { font-size:15px; font-weight:600; margin:0 0 4px; }
    :root[data-theme] form { gap:12px; }
    :root[data-theme] label { gap:4px; }
    :root[data-theme] .required,:root[data-theme] .summary-version { font-size:11px; }
    :root[data-theme] .metadata,:root[data-theme] .choices { gap:8px; grid-template-columns:repeat(auto-fit,minmax(min(100%,145px),1fr)); }
    :root[data-theme] .choice { gap:7px; padding:7px 8px; min-height:0; background:var(--soft); }
    :root[data-theme] .choice:has(:checked) { background:var(--tint); }
    :root[data-theme] .choice:has(:checked) small { color:var(--ink); }
    :root[data-theme] .choice input,:root[data-theme] .consent input { width:13px; height:13px; margin-top:3px; }
    :root[data-theme] .choice small { font-size:11px; }
    :root[data-theme] input[type=text],:root[data-theme] textarea,:root[data-theme] select { padding:6px 8px; }
    :root[data-theme] .choice,:root[data-theme] input[type=text],:root[data-theme] textarea,:root[data-theme] select,:root[data-theme] button,:root[data-theme] .primary-link,:root[data-theme] details,:root[data-theme] .publication,:root[data-theme] .error { border-radius:6px; }
    :root[data-theme] details,:root[data-theme] .publication { padding:8px 10px; border:1px solid var(--line); }
    :root[data-theme] .publication p { font-size:12px; margin:2px 0 7px; }
    :root[data-theme] .consent { gap:7px; color:var(--ink); }
    :root[data-theme] button,:root[data-theme] .primary-link { padding:6px 14px; }
    :root[data-theme] .actions { padding-top:12px; border-top:1px solid var(--line); }
  </style>
</head>
<body>
<main>
  <header>
    ${theme ? "" : '<p class="brand">ColorYourModel</p>'}
    <h1>${issueNumber ? t.created : t.title}</h1>
    ${issueNumber || showForm ? `<p>${issueNumber ? t.createdHint : t.intro}</p>` : ""}
  </header>
  ${error ? `<p role="alert" class="error">${t[error]}</p>` : ""}
  ${issueNumber ? `
    <p><a class="primary-link" href="${issueList}/${issueNumber}">${t.viewIssue} #${issueNumber}</a></p>
    <p><a href="${escapeHtml(returnUrl)}">${t.again}</a></p>` : ""}
  ${showForm ? `
  <form action="${submitUrl}" method="post">
    <input type="hidden" name="lang" value="${lang}">
    <fieldset>
      <legend>${t.kind}</legend>
      <div class="choices">
        ${["bug", "feature"].map(choice => `<label class="choice">
          <input type="radio" name="kind" value="${choice}"${choice === kind ? " checked" : ""} aria-describedby="${choice}-hint">
          <span><span class="field-name">${t[choice]}</span><small id="${choice}-hint">${t[`${choice}Hint`]}</small></span>
        </label>`).join("")}
      </div>
    </fieldset>
    <label><span class="field-name">${t.subject}<span class="required" aria-hidden="true">${t.required}</span></span>
      <input type="text" name="title" required maxlength="${limits.title}" value="${escapeHtml(form.title)}" placeholder="${escapeHtml(t[`${kind}Title`])}" data-bug="${escapeHtml(t.bugTitle)}" data-feature="${escapeHtml(t.featureTitle)}">
    </label>
    <label><span class="field-name">${t.description}<span class="required" aria-hidden="true">${t.required}</span></span>
      <textarea name="description" rows="4" required maxlength="${limits.description}" placeholder="${escapeHtml(t[`${kind}Prompt`])}" data-bug="${escapeHtml(t.bugPrompt)}" data-feature="${escapeHtml(t.featurePrompt)}">${escapeHtml(form.description)}</textarea>
    </label>
    <details${form.steps || form.os ? " open" : ""}>
      <summary>${t.optional}<span class="summary-version">${form.version ? ` · ${escapeHtml(form.version)}` : ""}</span></summary>
      <div class="extra-fields">
        <label>${t.steps}
          <textarea name="steps" rows="3" maxlength="${limits.steps}" placeholder="${escapeHtml(t.stepsPrompt)}">${escapeHtml(form.steps)}</textarea>
        </label>
        <div class="metadata">
          <label>${t.version}
            <input type="text" name="version" maxlength="${limits.version}" value="${escapeHtml(form.version)}">
          </label>
          <label>${t.os}
            <select name="os">
              <option value="">${t.select}</option>
              ${systems.map(os => `<option value="${os}"${os === form.os ? " selected" : ""}>${os === "Other" ? t.other : os}</option>`).join("")}
            </select>
          </label>
        </div>
      </div>
    </details>
    <div class="publication">
      <strong>${t.publicTitle}</strong>
      <p id="public-hint">${t.publicHint}</p>
      <label class="consent"><input type="checkbox" name="consent" value="yes" aria-describedby="public-hint" required${form.consent ? " checked" : ""}> ${t.consent}</label>
    </div>
    <div>
      <div class="cf-turnstile" data-sitekey="${escapeHtml(siteKey)}" data-action="feedback" data-size="flexible"${theme ? ` data-theme="${theme}"` : ""}></div>
      <p class="verification-status" role="status" data-pending="${escapeHtml(t.challengePending)}"></p>
      <noscript>${t.noscript}</noscript>
    </div>
    <div class="actions">
      ${issueListLink}
      <button type="submit" data-pending="${t.submitting}">${t.submit}</button>
    </div>
  </form>
  <script nonce="${nonce}">
    const form = document.querySelector('form');
    const challenge = form.querySelector('.cf-turnstile');
    if (challenge.clientWidth < 300) challenge.dataset.size = 'compact';
    const button = form.querySelector('button[type="submit"]');
    const label = button.textContent;
    const verificationStatus = form.querySelector('.verification-status');
    form.addEventListener('change', ({ target }) => {
      if (target.name !== 'kind') return;
      for (const field of form.querySelectorAll('[data-bug]')) field.placeholder = field.dataset[target.value];
    });
    const version = form.elements.namedItem('version');
    version.addEventListener('input', () => {
      form.querySelector('.summary-version').textContent = version.value.trim() ? ' · ' + version.value.trim() : '';
    });
    form.addEventListener('submit', event => {
      if (button.disabled) { event.preventDefault(); return; }
      if (!form.elements.namedItem('cf-turnstile-response')?.value) {
        event.preventDefault();
        verificationStatus.textContent = verificationStatus.dataset.pending;
        return;
      }
      verificationStatus.textContent = '';
      button.disabled = true;
      button.textContent = button.dataset.pending;
    });
    window.addEventListener('pageshow', () => {
      button.disabled = false;
      button.textContent = label;
    });
  </script>` : ""}
  ${showForm ? "" : `<footer>${issueListLink}</footer>`}
</main>
</body>
</html>`;

  return new Response(html, { status, headers: {
    "Content-Type": "text/html; charset=utf-8",
    "Cache-Control": "no-store",
    "Content-Security-Policy": `default-src 'none'; script-src 'nonce-${nonce}' https://challenges.cloudflare.com; style-src 'nonce-${nonce}'; frame-src https://challenges.cloudflare.com; connect-src 'self' https://challenges.cloudflare.com; form-action 'self'; base-uri 'none'; frame-ancestors 'none'`,
    "Referrer-Policy": "same-origin",
    "X-Content-Type-Options": "nosniff",
  } });
}
