//! A short navigation page over existing evidence, without new defect attribution.
use crate::{Result, ir};
use serde_json::Value;
use std::{fs, path::Path};

fn display_snippet(raw: &str) -> String {
    raw.strip_prefix("Some(")
        .and_then(|s| s.strip_suffix(')'))
        .and_then(|s| serde_json::from_str::<String>(s).ok())
        .unwrap_or_else(|| raw.to_owned())
}

pub fn write(out: &Path) -> Result<()> {
    let report: Value = serde_json::from_slice(&fs::read(out.join("report.json"))?)?;
    let mut lines = vec!["# 本地审阅入口".to_owned(), String::new(),
        "先核对运行结果与证明结果，再查看目标、前提和诊断。未证明项需要区分契约缺口、证明指导不足、转换限制及目标不当。".into(),
        "完整记录见 [report.json](report.json)。实际收益待真人评价。".into()];
    if report["build_success"] == false {
        let (stage, diagnostic) = if report["failure_stage"] == "metadata" {
            ("Cargo 包信息读取失败", "metadata.stderr")
        } else {
            ("构建失败", "build.stderr")
        };
        lines.extend([String::new(), format!("{stage}，尚未检查 client。请查看 [诊断]({diagnostic})。修复环境后使用新的输出目录重跑。")]);
        if report["failure_stage"] != "metadata" {
            lines.push("完整编译消息见 [build.stdout](build.stdout)。".into());
            let messages = fs::read_to_string(out.join("build.stdout"))?;
            for line in messages.lines() {
                let Ok(message) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if message["reason"] != "compiler-message" || message["message"]["level"] != "error"
                {
                    continue;
                }
                lines.push(format!(
                    "\n- {}。",
                    message["message"]["message"].as_str().unwrap_or("编译错误")
                ));
                for span in message["message"]["spans"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|s| s["is_primary"] == true)
                {
                    if let (Some(name), Some(manifest)) = (
                        span["file_name"].as_str(),
                        message["manifest_path"]
                            .as_str()
                            .or_else(|| report["package_manifest"].as_str()),
                    ) {
                        let file = Path::new(manifest)
                            .parent()
                            .unwrap_or(Path::new("."))
                            .join(name);
                        lines.push(format!(
                            "  [源位置](<{}:{}>)。",
                            file.display(),
                            span["line_start"].as_u64().unwrap_or(1)
                        ));
                    }
                }
            }
        }
    }
    for (ti, target) in report["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(samples) = target["samples"].as_array() else {
            lines.push(format!(
                "\n目标 {} 未建立测试清单，状态 {}。见 [目标证据](target-{ti}/)。",
                target["target"]["name"], target["status"]
            ));
            continue;
        };
        if samples.is_empty() {
            lines.push(format!(
                "\n目标 {} 的注册测试清单为空。",
                target["target"]["name"]
            ));
        }
        for (si, sample) in samples.iter().enumerate() {
            let dir = format!("target-{ti}/sample-{si}");
            lines.push(format!(
                "\n## {}\n\n运行 {}；检查 {}。",
                sample["function"].as_str().unwrap_or("未知调用方"),
                sample["runtime"]["status"].as_str().unwrap_or("未运行"),
                sample["status"].as_str().unwrap_or("未知")
            ));
            let original = out.join(&dir).join("original.json");
            let records: Value = if original.is_file() {
                serde_json::from_slice(&fs::read(original)?)?
            } else {
                Value::Null
            };
            let declaration = sample.get("original_declaration").or_else(|| {
                records["records"]
                    .as_array()?
                    .iter()
                    .find(|r| r["kind"] == "decl")
            });
            let span = declaration.and_then(|d| d["span"].as_str());
            let context: Value = fs::read(out.join(format!("target-{ti}/context.json")))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or(Value::Null);
            let location = span
                .and_then(|span| {
                    let cwd = Path::new(context["cwd"].as_str()?);
                    let (file, coords) = ir::span(span, cwd).ok()?;
                    Some(format!("[{span}](<{}:{}>)", file.display(), coords[0]))
                })
                .unwrap_or_else(|| span.unwrap_or("尚未取得，见目标诊断").to_owned());
            lines.push(format!("- 源位置：{location}。"));
            let goals = records["records"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["kind"] == "assert_operand")
                .collect::<Vec<_>>();
            if goals.is_empty() {
                let target = if sample["helper_context"].is_object() {
                    "调用前提及 helper 中的断言，详见 helper_context"
                } else if sample["target"].is_object() {
                    "调用前提；本调用方未提取到测试断言"
                } else {
                    "尚未建立；请查看不支持原因和提取记录"
                };
                lines.push(format!("- 检查目标：{target}。"));
            } else {
                for goal in goals {
                    lines.push(format!(
                        "- 测试断言操作数：`{}`，位置 {}。",
                        display_snippet(goal["snippet"].as_str().unwrap_or("待提取")),
                        goal["assertion_span"].as_str().unwrap_or("未知")
                    ));
                }
            }
            let prerequisites = sample["target"]["existing_preconditions"].as_u64();
            lines.push(format!(
                "- 入口前提：{}；生成 helper 新增前提 {}。",
                prerequisites
                    .map(|n| n.to_string())
                    .unwrap_or("尚未确认".into()),
                sample["added_helper_preconditions"].as_u64().unwrap_or(0)
            ));
            for (file, label) in [
                ("contracts.json", "调用契约与前提来源"),
                ("helper-context.json", "helper 条件与断言"),
                ("check.json", "Verus 诊断"),
                ("original.json", "原始提取记录"),
            ] {
                if out.join(&dir).join(file).is_file() {
                    lines.push(format!("- [{label}]({dir}/{file})。"));
                }
            }
            lines.push("- 待确认：上述前提是否符合实际使用；失败是否需要补证明或修订 spec。证明通过仍依赖列出的契约与转换条件。".into());
            for obligation in sample["review"]["obligations"]
                .as_array()
                .into_iter()
                .flatten()
            {
                lines.push(format!(
                    "- 失败位置分类：{}；{}。",
                    obligation["kind"].as_str().unwrap_or("未分类"),
                    obligation["message"].as_str().unwrap_or("见诊断")
                ));
            }
            if let Some(reason) = sample["reason"]
                .as_str()
                .or_else(|| sample["error"].as_str())
            {
                lines.push(format!("- 阻塞原因：{reason}"));
            }
        }
    }
    fs::write(out.join("REVIEW.md"), lines.join("\n") + "\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_failure_links_the_compiler_message_package() {
        let root = std::env::temp_dir().join(format!(
            "spec-reader-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("report.json"),
            json!({"build_success": false,
            "package_manifest": "/client/Cargo.toml"})
            .to_string(),
        )
        .unwrap();
        fs::write(
            root.join("build.stdout"),
            json!({"reason": "compiler-message",
            "manifest_path": "/dependency/Cargo.toml", "message": {"level": "error",
            "message": "missing type", "spans": [{"is_primary":true,
                "file_name":"src/lib.rs", "line_start":7}]}})
            .to_string(),
        )
        .unwrap();
        write(&root).unwrap();
        let text = fs::read_to_string(root.join("REVIEW.md")).unwrap();
        assert!(text.contains("/dependency/src/lib.rs:7"));
        assert!(text.contains("build.stdout") && text.contains("尚未检查 client"));
        assert!(!text.contains("/client/src/lib.rs"));
        fs::remove_dir_all(root).unwrap();
    }
}
