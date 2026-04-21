use ironflow_engine::config::{AgentStepConfig, HttpConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo};

pub struct WeatherReport;

impl WorkflowHandler for WeatherReport {
    fn name(&self) -> &str {
        "weather-report"
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Fetches live weather data for Paris via HTTP, \
                          then asks an AI agent to produce a human-readable summary."
                .to_string(),
            source_code: Some(include_str!("weather_report.rs").to_string()),
            sub_workflows: Vec::new(),
            category: None,
            version: self.version().to_string(),
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            // Step 1: Fetch raw weather data via HTTP
            let weather = ctx
                .http(
                    "fetch-weather",
                    HttpConfig::get("https://wttr.in/Paris?format=j1"),
                )
                .await?;

            // Extract the raw JSON body from the HTTP response
            let body = weather
                .output
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");

            // Step 2: Ask an AI agent to summarize the weather data
            ctx.agent(
                "summarize",
                AgentStepConfig::new(&format!(
                    "Here is raw weather JSON data for Paris:\n\n{}\n\n\
                     Write a concise, human-readable weather report for Paris. \
                     Include: temperature, weather description, humidity, wind speed, \
                     and feels-like temperature. Format it nicely with sections. \
                     Keep it under 10 lines.",
                    &body[..body.floor_char_boundary(3000)]
                ))
                .model("haiku")
                .max_budget_usd(0.05)
                .max_turns(1),
            )
            .await?;

            Ok(())
        })
    }
}
