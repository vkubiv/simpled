using core.Libs.EntityAccess;
using core.Primitive;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Operations;

public static class OrgMemberOps
{
    public static Task<OrgMemberEntity?> TryGet(int organizationId, int accountId, Read<OrgMemberEntity> readMembers) =>
        readMembers.FirstOrDefaultAsync(m => m.OrganizationId == organizationId && m.AccountId == accountId);

    public static async Task<OrgMemberEntity> Add(
        int organizationId,
        AccountEntity account,
        OrgRole role,
        DateTime now,
        Mutate<OrgMemberEntity> mutateMembers)
    {
        var member = new OrgMemberEntity
        {
            OrganizationId = organizationId,
            Account = account,
            Role = OrgRoles.ToValue(role),
            JoinedAt = now,
        };
        await mutateMembers.AddAsync(member);
        return member;
    }

    public static Task<int> CountOwners(int organizationId, Read<OrgMemberEntity> readMembers)
    {
        var owner = OrgRoles.ToValue(OrgRole.Owner);
        return readMembers.CountAsync(m => m.OrganizationId == organizationId && m.Role == owner);
    }

    /// <summary>Removes the membership and the account's places in the organization's teams.</summary>
    public static async Task Remove(
        OrgMemberEntity member,
        Mutate<OrgMemberEntity> mutateMembers,
        Mutate<TeamMemberEntity> mutateTeamMembers,
        Read<TeamEntity> readTeams)
    {
        var teamMembers = await mutateTeamMembers.Read
            .Where(tm => tm.AccountId == member.AccountId)
            .Join(readTeams.Where(t => t.OrganizationId == member.OrganizationId),
                tm => tm.TeamId, t => t.Id, (tm, _) => tm)
            .ToListAsync();
        mutateTeamMembers.RemoveRange(teamMembers);
        mutateMembers.Remove(member);
    }
}
