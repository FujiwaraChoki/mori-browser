#!/usr/bin/env python3
"""Regenerates linux/patches/*.patch from the Chromium tree: each patch is
the diff between ungoogled-chromium's version of its files (see
pristine.py) and the tree.

  linux/tools/mkpatch.py          # all patches
  linux/tools/mkpatch.py 0004     # one
"""
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "ungoogled-chromium-linux" / "build" / "src"
PRISTINE = ROOT / "ungoogled-chromium-linux" / "pristine"
PATCHES = ROOT / "linux" / "patches"

# Which Chromium files each patch covers.
GROUPS = {
    "0001-drop-unreachable-managed-profile-test-path": [
        "chrome/browser/enterprise/signin/profile_management_disclaimer_service.cc",
    ],
    "0002-mori-hooks": [
        "chrome/browser/ui/browser_command_controller.cc",
        "chrome/browser/ui/accelerator_table.cc",
        "chrome/browser/ui/BUILD.gn",
        "chrome/browser/ui/color/BUILD.gn",
        "chrome/browser/ui/color/chrome_color_mixers.cc",
    ],
    "0003-mori-profile-and-policy-dirs": [
        "chrome/common/chrome_paths_linux.cc",
        "components/policy/core/common/policy_paths.cc",
    ],
    "0004-mori-window-layout": [
        "chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc",
        "chrome/browser/ui/views/toolbar/toolbar_view.cc",
        "chrome/browser/ui/views/frame/browser_frame_view_linux.cc",
        "chrome/browser/ui/views/frame/vertical_tab_strip_region_view.cc",
        "chrome/browser/ui/views/location_bar/location_bar_view.cc",
        "chrome/browser/ui/layout_constants.cc",
        "chrome/browser/ui/views/chrome_typography.cc",
    ],
    "0005-mori-default-prefs": [
        "chrome/browser/ui/tabs/tab_strip_prefs.cc",
        "chrome/browser/ui/tabs/saved_tab_groups/saved_tab_group_pref_names.cc",
        "components/bookmarks/browser/bookmark_utils.cc",
        "components/search_engines/template_url_prepopulate_data.cc",
        "chrome/browser/prefs/session_startup_pref.cc",
        "chrome/browser/ui/toolbar/chrome_location_bar_model_delegate.cc",
        "chrome/browser/profiles/profile.cc",
    ],
    "0006-mori-spaces": [
        "chrome/browser/ui/ui_features.cc",
        "chrome/browser/ui/tabs/tab_strip_model.cc",
    ],
    "0007-mori-sidebar-rows": [
        "chrome/browser/ui/tabs/vertical_tab_strip_state.h",
        "chrome/browser/ui/views/tabs/common/tab_strip_view.cc",
        "chrome/browser/ui/views/tabs/common/tab_strip_view_layout.cc",
        "chrome/browser/ui/views/tabs/common/tab_group_view_layout.cc",
        "chrome/browser/ui/views/tabs/common/unpinned_tab_container_view_layout.cc",
        "chrome/browser/ui/views/tabs/common/tab_view_vertical_layout.cc",
        "chrome/browser/ui/tabs/vertical_tab_strip_state_controller.cc",
        "chrome/browser/ui/views/tabs/vertical/vertical_tab_strip_bottom_container.cc",
        "chrome/browser/ui/views/bookmarks/bookmark_page_action_controller.cc",
    ],
    "0008-mori-lucide-icons": [
        "ui/gfx/paint_vector_icon.cc",
        "ui/gfx/BUILD.gn",
    ],
    "0009-mori-menus": [
        "ui/views/controls/menu/menu_config.cc",
        "chrome/browser/ui/toolbar/app_menu_model.cc",
    ],
    "0011-capture-overlay-offset": [
        "chrome/browser/image_editor/screenshot_flow.cc",
    ],
    "0013-mori-tab-names": [
        "chrome/browser/ui/tab_ui_helper.cc",
        "chrome/browser/ui/browser_tabrestore.cc",
        "chrome/browser/sessions/session_service_base.cc",
        "chrome/browser/ui/views/tabs/common/tab_view.cc",
    ],
    "0014-mori-boosts": [
        "chrome/browser/ui/tab_helpers.cc",
    ],
    "0015-mori-space-wash": [
        "chrome/browser/ui/views/frame/themed_background.cc",
    ],
    "0016-mori-links-from-other-apps": [
        "chrome/browser/ui/startup/startup_browser_creator_impl.cc",
    ],
    "0017-import-from-zen": [
        "chrome/browser/importer/importer_list.cc",
    ],
    "0012-bubbles-stay-in-window": [
        "ui/views/bubble/bubble_dialog_delegate_view.cc",
    ],
    "0010-mori-peek": [
        "third_party/blink/renderer/core/loader/navigation_policy.cc",
        "chrome/browser/ui/browser_web_contents_delegate/browser_web_contents_delegate.cc",
    ],
    "0019-mori-overlay-scrollbars": [
        "ui/native_theme/features/native_theme_features.cc",
    ],
    "0020-mori-find-bar": [
        "chrome/browser/ui/views/frame/browser_view.cc",
    ],
    "0021-mori-quiet-address-field": [
        "chrome/browser/ui/views/location_bar/cookie_controls/cookie_controls_page_action_controller.cc",
    ],
    "0022-mori-placeholder-favicon": [
        "chrome/browser/favicon/favicon_utils.cc",
    ],
    "0018-mori-context-menus": [
        "chrome/browser/ui/views/tabs/common/tab_strip_collection_controller.cc",
        "chrome/browser/ui/views/frame/browser_widget.cc",
        "chrome/browser/renderer_context_menu/render_view_context_menu.cc",
        "chrome/browser/ui/views/renderer_context_menu/render_view_context_menu_views.cc",
    ],
}


def main():
    wanted = sys.argv[1:]
    for name, files in GROUPS.items():
        if wanted and not any(name.startswith(w) for w in wanted):
            continue
        out = []
        for f in files:
            if not (PRISTINE / f).exists():
                sys.exit(f"no pristine copy of {f}: run linux/tools/pristine.py {f}")
            diff = subprocess.run(
                ["diff", "-u", "--label", f"a/{f}", "--label", f"b/{f}",
                 str(PRISTINE / f), str(SRC / f)],
                capture_output=True, text=True)
            out.append(diff.stdout)
        (PATCHES / f"{name}.patch").write_text("".join(out))
        print(f"{name}.patch: {sum(t.count(chr(10)) for t in out)} lines")


if __name__ == "__main__":
    main()
