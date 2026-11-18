(function () {
    function formatThreadTime(date) {
        const pad = (value) => String(value).padStart(2, "0");
        const year = String(date.getFullYear()).slice(-2);
        return `${pad(date.getDate())}.${pad(date.getMonth() + 1)}.${year}-${pad(date.getHours())}:${pad(date.getMinutes())}`;
    }

    function formatLocalTime(node) {
        const raw = node.getAttribute("data-utc");
        if (!raw) return;
        const date = new Date(raw);
        if (Number.isNaN(date.getTime())) return;

        const requested = node.getAttribute("data-format") || "";
        if (requested === "DD.MM.YY-HH:MM") {
            node.textContent = formatThreadTime(date);
            return;
        }

        const timeFormatter = new Intl.DateTimeFormat(undefined, {
            hour: "2-digit",
            minute: "2-digit",
            hour12: false,
        });
        const dateFormatter = new Intl.DateTimeFormat(undefined, {
            month: "short",
            day: "numeric",
        });
        const dayStart = (value) =>
            new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();

        const now = new Date();
        const diffDays = Math.round((dayStart(now) - dayStart(date)) / 86400000);
        if (diffDays === 0) {
            node.textContent = timeFormatter.format(date);
        } else if (diffDays === 1) {
            node.textContent = "yesterday";
        } else {
            node.textContent = dateFormatter.format(date).replace(",", "");
        }
    }

    function installMentionButtons() {
        const input = document.getElementById("body");
        if (!input) return;
        document.querySelectorAll(".mention-insert").forEach((button) => {
            button.addEventListener("click", () => {
                const handle = button.getAttribute("data-user");
                if (!handle) return;
                const prefix =
                    input.value.length > 0 && !/\s$/.test(input.value) ? " " : "";
                input.value = `${input.value}${prefix}@${handle} `;
                input.focus();
            });
        });
    }

    function highlightActiveRoute() {
        const path = window.location.pathname;
        document.querySelectorAll(".nav-link[data-route]").forEach((node) => {
            const route = node.getAttribute("data-route");
            if (!route) return;
            if ((route === "/" && path === "/") || (route !== "/" && path.startsWith(route))) {
                node.classList.add("is-active");
            }
        });
    }

    function installComposerPreviews() {
        document.querySelectorAll("[data-preview-toggle]").forEach((button) => {
            const previewId = button.getAttribute("data-preview-toggle");
            const output = previewId ? document.getElementById(previewId) : null;
            const input = document.querySelector(`[data-preview-source="${previewId}"]`);
            if (!output || !input) return;

            button.addEventListener("click", () => {
                const isHidden = output.hasAttribute("hidden");
                if (isHidden) {
                    output.textContent = input.value.trim() || "Nothing to preview.";
                    output.removeAttribute("hidden");
                    button.textContent = "Hide preview";
                } else {
                    output.setAttribute("hidden", "");
                    button.textContent = "Preview";
                }
            });

            input.addEventListener("input", () => {
                if (!output.hasAttribute("hidden")) {
                    output.textContent = input.value.trim() || "Nothing to preview.";
                }
            });
        });
    }

    function installAccountMenu() {
        const root = document.querySelector("[data-account-menu]");
        if (!root) return;
        const button = root.querySelector("[data-account-menu-button]");
        const panel = root.querySelector("[data-account-menu-panel]");
        if (!button || !panel) return;

        const isOpen = () => !panel.hasAttribute("hidden");
        const close = () => {
            panel.setAttribute("hidden", "");
            root.classList.remove("is-open");
            button.setAttribute("aria-expanded", "false");
        };
        const open = () => {
            panel.removeAttribute("hidden");
            root.classList.add("is-open");
            button.setAttribute("aria-expanded", "true");
        };

        button.addEventListener("click", (event) => {
            event.stopPropagation();
            if (isOpen()) {
                close();
            } else {
                open();
            }
        });

        panel.addEventListener("click", (event) => event.stopPropagation());
        document.addEventListener("click", () => {
            if (isOpen()) close();
        });
        document.addEventListener("keydown", (event) => {
            if (event.key === "Escape" && isOpen()) {
                close();
                button.focus();
            }
        });
    }

    function installLiveStatePolling() {
        const badge = document.querySelector("[data-live-notification-badge]");
        if (!badge || !window.fetch) return;

        const apply = (count) => {
            const next = Number(count) || 0;
            badge.textContent = String(next);
            if (next > 0) {
                badge.removeAttribute("hidden");
            } else {
                badge.setAttribute("hidden", "");
            }
        };

        const refresh = () => {
            fetch("/live/state", {
                headers: { accept: "application/json" },
                credentials: "same-origin",
                cache: "no-store",
            })
                .then((response) => (response.ok ? response.json() : null))
                .then((state) => {
                    if (state) apply(state.unread_notifications);
                })
                .catch(() => {});
        };

        window.setInterval(refresh, 20000);
        refresh();
    }

    document.addEventListener("DOMContentLoaded", () => {
        document.querySelectorAll(".local-datetime[data-utc]").forEach(formatLocalTime);
        installMentionButtons();
        highlightActiveRoute();
        installComposerPreviews();
        installAccountMenu();
        installLiveStatePolling();
    });
})();

// Drawer toggle (M1+M2 responsive shell).
(function () {
    const button = document.querySelector('[data-drawer-toggle]');
    if (!button) return;
    const body = document.body;
    const close = () => body.removeAttribute('data-drawer-open');
    button.addEventListener('click', () => {
        if (body.hasAttribute('data-drawer-open')) close();
        else body.setAttribute('data-drawer-open', '');
    });
    document.querySelectorAll('.app-sidebar a').forEach(a =>
        a.addEventListener('click', close)
    );
    document.addEventListener('keydown', (e) => { if (e.key === 'Escape') close(); });
    document.addEventListener('click', (e) => {
        if (!body.hasAttribute('data-drawer-open')) return;
        const sidebar = document.querySelector('.app-sidebar');
        if (sidebar && !sidebar.contains(e.target) && !button.contains(e.target)) close();
    });
})();
