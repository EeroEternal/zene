use crate::bash::BashTool;
use crate::edit::EditTool;
use crate::glob::GlobTool;
use crate::grep::GrepTool;
use crate::plan::{EnterPlanModeTool, ExitPlanModeTool};
use crate::read::ReadTool;
use crate::registry::ToolRegistry;
use crate::skill::SkillTool;
use crate::subagent::SubagentProfile;
use crate::task::TaskTool;
use crate::write::WriteTool;

pub fn builtin_tools() -> ToolRegistry {
    ToolRegistry::new(all_builtin_tool_boxes())
}

pub fn tools_for_profile(profile: SubagentProfile) -> ToolRegistry {
    match profile {
        SubagentProfile::Explore => ToolRegistry::new(vec![
            Box::new(ReadTool),
            Box::new(GrepTool),
            Box::new(GlobTool),
        ]),
        SubagentProfile::Coder => ToolRegistry::new(vec![
            Box::new(ReadTool),
            Box::new(WriteTool),
            Box::new(EditTool),
            Box::new(BashTool),
            Box::new(GrepTool),
            Box::new(GlobTool),
        ]),
    }
}

fn all_builtin_tool_boxes() -> Vec<Box<dyn crate::registry::Tool>> {
    vec![
        Box::new(ReadTool),
        Box::new(WriteTool),
        Box::new(EditTool),
        Box::new(BashTool),
        Box::new(GrepTool),
        Box::new(GlobTool),
        Box::new(SkillTool),
        Box::new(TaskTool),
        Box::new(EnterPlanModeTool),
        Box::new(ExitPlanModeTool),
    ]
}
