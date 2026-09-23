use std::collections::HashSet;
use leptos::prelude::*;
use crate::components::section_heading::SubHeading;
use crate::models::{DiffLineInfo, DiffTag};

const CONTEXT_LINES: usize = 3;

#[derive(Clone)]
struct NumberedDiffLine {
    old_num: Option<usize>,
    new_num: Option<usize>,
    tag: DiffTag,
    content: String,
}

fn assign_line_numbers(lines: Vec<DiffLineInfo>) -> Vec<NumberedDiffLine> {
    let mut old_num: usize = 0;
    let mut new_num: usize = 0;
    lines
        .into_iter()
        .map(|line| {
            let (old, new) = match line.tag {
                DiffTag::Equal => {
                    old_num += 1;
                    new_num += 1;
                    (Some(old_num), Some(new_num))
                }
                DiffTag::Delete => {
                    old_num += 1;
                    (Some(old_num), None)
                }
                DiffTag::Insert => {
                    new_num += 1;
                    (None, Some(new_num))
                }
            };
            NumberedDiffLine {
                old_num: old,
                new_num: new,
                tag: line.tag,
                content: line.content,
            }
        })
        .collect()
}

fn compute_sections(
    lines: Vec<NumberedDiffLine>,
    context: usize,
) -> Vec<(bool, Vec<NumberedDiffLine>)> {
    if lines.is_empty() {
        return vec![];
    }

    let mut visible = vec![false; lines.len()];
    for (i, line) in lines.iter().enumerate() {
        if !matches!(line.tag, DiffTag::Equal) {
            let start = i.saturating_sub(context);
            let end = (i + context + 1).min(lines.len());
            for v in &mut visible[start..end] {
                *v = true;
            }
        }
    }

    if visible.iter().all(|&v| v) {
        return vec![(true, lines)];
    }

    let mut sections = Vec::new();
    let mut current_visible = visible[0];
    let mut current_lines = Vec::new();

    for (line, &vis) in lines.into_iter().zip(visible.iter()) {
        if vis != current_visible && !current_lines.is_empty() {
            sections.push((current_visible, std::mem::take(&mut current_lines)));
            current_visible = vis;
        }
        current_lines.push(line);
    }
    if !current_lines.is_empty() {
        sections.push((current_visible, current_lines));
    }

    sections
}

fn render_diff_line(line: &NumberedDiffLine) -> impl IntoView {
    let (bg_class, prefix) = match line.tag {
        DiffTag::Insert => ("bg-emerald-50 dark:bg-emerald-900/20", "+"),
        DiffTag::Delete => ("bg-red-50 dark:bg-red-900/20", "-"),
        DiffTag::Equal => ("", " "),
    };
    let old_str = line.old_num.map(|n| n.to_string()).unwrap_or_default();
    let new_str = line.new_num.map(|n| n.to_string()).unwrap_or_default();
    let content = line.content.clone();
    view! {
        <div class={format!("flex {bg_class} text-gray-800 dark:text-gray-200")}>
            <span class="w-10 text-right pr-2 text-gray-400 dark:text-gray-500 select-none border-r border-gray-200 dark:border-gray-700 shrink-0">
                {old_str}
            </span>
            <span class="w-10 text-right pr-2 text-gray-400 dark:text-gray-500 select-none border-r border-gray-200 dark:border-gray-700 shrink-0">
                {new_str}
            </span>
            <span class="pl-2 whitespace-pre flex-1">{prefix}" "{content}</span>
        </div>
    }
}

#[component]
fn CollapsedSection(
    idx: usize,
    lines: Vec<NumberedDiffLine>,
    expanded: ReadSignal<HashSet<usize>>,
    set_expanded: WriteSignal<HashSet<usize>>,
) -> impl IntoView {
    let count = lines.len();
    let stored = StoredValue::new(lines);
    move || {
        if expanded.get().contains(&idx) {
            stored
                .get_value()
                .iter()
                .map(render_diff_line)
                .collect::<Vec<_>>()
                .into_any()
        } else {
            view! {
                <div
                    class="flex items-center justify-center py-1 bg-sky-50 dark:bg-sky-900/20 text-sky-600 dark:text-sky-400 cursor-pointer hover:bg-sky-100 dark:hover:bg-sky-900/30 select-none border-y border-sky-200/50 dark:border-sky-800/50 text-xs"
                    on:click=move |_| {
                        set_expanded.update(|s| { s.insert(idx); });
                    }
                >
                    "⋯ " {count.to_string()} " lines hidden ⋯"
                </div>
            }
            .into_any()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(s: &str) -> DiffLineInfo {
        DiffLineInfo {
            tag: DiffTag::Equal,
            content: s.to_string(),
        }
    }

    fn ins(s: &str) -> DiffLineInfo {
        DiffLineInfo {
            tag: DiffTag::Insert,
            content: s.to_string(),
        }
    }

    fn del(s: &str) -> DiffLineInfo {
        DiffLineInfo {
            tag: DiffTag::Delete,
            content: s.to_string(),
        }
    }

    #[test]
    fn line_numbers_equal_lines() {
        let lines = vec![eq("a"), eq("b"), eq("c")];
        let numbered = assign_line_numbers(lines);
        assert_eq!(numbered.len(), 3);
        assert_eq!(numbered[0].old_num, Some(1));
        assert_eq!(numbered[0].new_num, Some(1));
        assert_eq!(numbered[2].old_num, Some(3));
        assert_eq!(numbered[2].new_num, Some(3));
    }

    #[test]
    fn line_numbers_insert_only_advances_new() {
        let lines = vec![eq("a"), ins("b"), eq("c")];
        let numbered = assign_line_numbers(lines);
        assert_eq!(numbered[0].old_num, Some(1));
        assert_eq!(numbered[0].new_num, Some(1));
        assert_eq!(numbered[1].old_num, None);
        assert_eq!(numbered[1].new_num, Some(2));
        assert_eq!(numbered[2].old_num, Some(2));
        assert_eq!(numbered[2].new_num, Some(3));
    }

    #[test]
    fn line_numbers_delete_only_advances_old() {
        let lines = vec![eq("a"), del("b"), eq("c")];
        let numbered = assign_line_numbers(lines);
        assert_eq!(numbered[1].old_num, Some(2));
        assert_eq!(numbered[1].new_num, None);
        assert_eq!(numbered[2].old_num, Some(3));
        assert_eq!(numbered[2].new_num, Some(2));
    }

    #[test]
    fn empty_input_produces_no_sections() {
        let sections = compute_sections(assign_line_numbers(vec![]), 3);
        assert!(sections.is_empty());
    }

    #[test]
    fn small_diff_all_visible() {
        let lines = vec![eq("a"), ins("b"), eq("c")];
        let sections = compute_sections(assign_line_numbers(lines), 3);
        assert_eq!(sections.len(), 1);
        assert!(sections[0].0, "single section should be visible");
        assert_eq!(sections[0].1.len(), 3);
    }

    #[test]
    fn large_unchanged_region_gets_collapsed() {
        let mut lines = Vec::new();
        for i in 0..20 {
            lines.push(eq(&format!("line {i}")));
        }
        lines.push(ins("new line"));
        for i in 21..40 {
            lines.push(eq(&format!("line {i}")));
        }

        let sections = compute_sections(assign_line_numbers(lines), 3);
        // Should have: collapsed prefix, visible hunk, collapsed suffix
        assert_eq!(sections.len(), 3);
        assert!(!sections[0].0, "leading unchanged should be collapsed");
        assert!(sections[1].0, "hunk should be visible");
        assert!(!sections[2].0, "trailing unchanged should be collapsed");

        // Context: 3 lines before change + change + 3 lines after
        assert_eq!(sections[1].1.len(), 7);
        // Leading collapsed: 20 - 3 = 17 lines
        assert_eq!(sections[0].1.len(), 17);
        // Trailing collapsed: 19 - 3 = 16 lines
        assert_eq!(sections[2].1.len(), 16);
    }

    #[test]
    fn two_changes_with_gap_produce_separate_hunks() {
        let mut lines = Vec::new();
        lines.push(ins("first change"));
        for _ in 0..20 {
            lines.push(eq("unchanged"));
        }
        lines.push(del("second change"));

        let sections = compute_sections(assign_line_numbers(lines), 3);
        // visible hunk, collapsed middle, visible hunk
        assert_eq!(sections.len(), 3);
        assert!(sections[0].0);
        assert!(!sections[1].0);
        assert!(sections[2].0);
    }

    #[test]
    fn adjacent_changes_merge_context() {
        let mut lines = Vec::new();
        lines.push(ins("a"));
        // 5 equal lines between changes — within 2*context (6), so no collapse
        for i in 0..5 {
            lines.push(eq(&format!("mid {i}")));
        }
        lines.push(del("b"));

        let sections = compute_sections(assign_line_numbers(lines), 3);
        // Everything within context of one or both changes -> single visible section
        assert_eq!(sections.len(), 1);
        assert!(sections[0].0);
    }

    #[test]
    fn all_equal_lines_collapsed_when_no_changes() {
        let lines: Vec<_> = (0..10).map(|i| eq(&format!("line {i}"))).collect();
        let sections = compute_sections(assign_line_numbers(lines), 3);
        assert_eq!(sections.len(), 1);
        assert!(!sections[0].0, "no changes means nothing to anchor context, all collapsed");
        assert_eq!(sections[0].1.len(), 10);
    }
}

/// Renders a GitHub-style collapsible diff with line numbers.
#[component]
pub fn DiffView(lines: Vec<DiffLineInfo>) -> impl IntoView {
    let additions = lines
        .iter()
        .filter(|l| matches!(l.tag, DiffTag::Insert))
        .count();
    let deletions = lines
        .iter()
        .filter(|l| matches!(l.tag, DiffTag::Delete))
        .count();

    let numbered = assign_line_numbers(lines);
    let sections = compute_sections(numbered, CONTEXT_LINES);
    let collapsed_count = sections.iter().filter(|(vis, _)| !vis).count();
    let (expanded, set_expanded) = signal(HashSet::<usize>::new());

    let mut collapsed_idx = 0usize;
    let section_views: Vec<_> = sections
        .into_iter()
        .map(|(is_visible, section_lines)| {
            if is_visible {
                section_lines
                    .iter()
                    .map(render_diff_line)
                    .collect::<Vec<_>>()
                    .into_any()
            } else {
                let idx = collapsed_idx;
                collapsed_idx += 1;
                view! {
                    <CollapsedSection
                        idx=idx
                        lines=section_lines
                        expanded=expanded
                        set_expanded=set_expanded
                    />
                }
                .into_any()
            }
        })
        .collect();

    view! {
        <SubHeading>"Changes (compared to next version)"</SubHeading>
        <div class="flex items-center gap-4 mb-2">
            <p class="text-sm text-gray-500 dark:text-gray-400">
                <span class="text-emerald-500">"+" {additions.to_string()} " added"</span>
                " "
                <span class="text-red-500">"-" {deletions.to_string()} " removed"</span>
            </p>
            {if collapsed_count > 0 {
                Some(view! {
                    <button
                        class="text-xs text-sky-600 dark:text-sky-400 hover:underline cursor-pointer"
                        on:click=move |_| {
                            set_expanded.update(|s| {
                                for i in 0..collapsed_count {
                                    s.insert(i);
                                }
                            });
                        }
                    >
                        "Expand all"
                    </button>
                })
            } else {
                None
            }}
        </div>
        <div class="bg-gray-50 dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg overflow-x-auto text-xs font-mono mb-6">
            {section_views}
        </div>
    }
}
