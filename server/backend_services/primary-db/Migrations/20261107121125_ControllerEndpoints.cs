using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace primary_db.Migrations
{
    /// <inheritdoc />
    public partial class ControllerEndpoints : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.AddColumn<string>(
                name: "health_url",
                table: "deployments",
                type: "text",
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "version_url",
                table: "deployments",
                type: "text",
                nullable: true);

            // What the monitor asked before; the next sync moves Docker deployments to their controller.
            migrationBuilder.Sql(
                "UPDATE deployments SET version_url = url || '/.well-known/simpled/version' WHERE url IS NOT NULL;");
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropColumn(
                name: "health_url",
                table: "deployments");

            migrationBuilder.DropColumn(
                name: "version_url",
                table: "deployments");
        }
    }
}
