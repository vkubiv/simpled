using core.Auth;
using core.Libs.EntityAccess;
using core.Operations;
using core.Primitive;
using db.Entities;
using main_svc.Auth;
using primary_db;

namespace utest.TestSupport;

public static class OrgSupport
{
    public static MainClaims Claims(AccountEntity account) =>
        new() { AccountId = account.Id, SessionId = 1, AuthType = AuthType.CookieSession };

    /// <summary>An organization owned by a new account, ann@test.com by default.</summary>
    public static async Task<(OrganizationEntity Organization, AccountEntity Owner)> CreateOrg(PrimaryDbContext db,
        string name = "Acme", string ownerEmail = "ann@test.com")
    {
        var owner = await AccountSupport.CreateAccount(db, ownerEmail);
        var (organization, _) = await OrganizationOps.Create(name, owner, DateTime.UtcNow,
            mutateOrganizations: new Mutate<OrganizationEntity>(db),
            mutateMembers: new Mutate<OrgMemberEntity>(db));
        await db.SaveChangesAsync();
        return (organization, owner);
    }

    /// <summary>A new account <paramref name="email"/> joined to the organization with <paramref name="role"/>.</summary>
    public static async Task<AccountEntity> AddMember(PrimaryDbContext db, OrganizationEntity organization,
        string email, OrgRole role)
    {
        var account = await AccountSupport.CreateAccount(db, email);
        await OrgMemberOps.Add(organization.Id, account, role, DateTime.UtcNow, new Mutate<OrgMemberEntity>(db));
        await db.SaveChangesAsync();
        return account;
    }

    public static async Task<TeamEntity> CreateTeam(PrimaryDbContext db, OrganizationEntity organization,
        string name, params AccountEntity[] members)
    {
        var team = await TeamOps.Create(organization.Id, name, DateTime.UtcNow, new Mutate<TeamEntity>(db));
        foreach (var member in members)
        {
            await TeamOps.AddMember(team, member, DateTime.UtcNow, new Mutate<TeamMemberEntity>(db));
        }

        await db.SaveChangesAsync();
        return team;
    }

    public static async Task<ProjectEntity> CreateProject(PrimaryDbContext db, OrganizationEntity organization,
        string name, params TeamEntity[] teams)
    {
        var project = await ProjectOps.Create(organization.Id, name, null, teams, DateTime.UtcNow,
            mutateProjects: new Mutate<ProjectEntity>(db),
            mutateProjectTeams: new Mutate<ProjectTeamEntity>(db));
        await db.SaveChangesAsync();
        return project;
    }

    public static string RoleOf(PrimaryDbContext db, OrganizationEntity organization, AccountEntity account) =>
        db.OrgMembers.Single(m => m.OrganizationId == organization.Id && m.AccountId == account.Id).Role;
}
