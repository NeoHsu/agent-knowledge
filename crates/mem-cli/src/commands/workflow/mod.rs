mod listing;
mod record;
mod scaffold;
mod show;
mod validate;

use super::*;

pub(crate) fn cmd_workflow(app: &App, command: WorkflowCommand) -> Result<()> {
    app.require_schema()?;
    let writes_store = matches!(&command, WorkflowCommand::Record(_))
        || matches!(&command, WorkflowCommand::Show(args) if args.with_graph_context);
    let conn = if writes_store {
        app.conn()?
    } else {
        app.read_conn()?
    };
    match command {
        WorkflowCommand::List(args) => listing::list(app, &conn, args),
        WorkflowCommand::Show(args) => show::show(app, &conn, args),
        WorkflowCommand::Find(args) => listing::find(app, &conn, args),
        WorkflowCommand::New(args) => scaffold::scaffold(args),
        WorkflowCommand::Record(args) => record::record(&conn, args),
        WorkflowCommand::Validate(args) => validate::validate(app, &conn, args),
    }
}
