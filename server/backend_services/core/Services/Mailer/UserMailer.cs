namespace core.Services.Mailer;

public class UserMailConfig
{
    public required string PostMasterEmail { get; set; }

    /// <summary>Web app base URL with a trailing slash; the links below are pages of the web app.</summary>
    public required string WebAppUrl { get; set; }
}

public class UserMailer
{
    private const string Language = "en";

    private readonly IMailer _mailer;
    private readonly UserMailConfig _mailConfig;

    public UserMailer(IMailer mailer, UserMailConfig mailConfig)
    {
        _mailer = mailer;
        _mailConfig = mailConfig;
    }

    public Task SendEmailConfirmation(string email, string name, string token) =>
        Send(email, name, "email_confirmation", Link("confirm-email", token));

    public Task SendResetPassword(string email, string name, string token) =>
        Send(email, name, "reset_password", Link("reset-password", token));

    public Task SendOrgInvitation(string email, string organizationName, string inviterName, string role,
        string token) =>
        Send(email, email, "org_invitation", Link("invitation", token), new Dictionary<string, object>
        {
            { "organization", organizationName },
            { "inviter", inviterName },
            { "role", role },
        });

    private string Link(string page, string token) =>
        $"{_mailConfig.WebAppUrl}{page}?token={Uri.EscapeDataString(token)}";

    private Task Send(string email, string name, string templateName, string link,
        Dictionary<string, object>? extra = null)
    {
        var data = new Dictionary<string, object>
        {
            { "name", name },
            { "email", email },
            { "link", link },
        };
        foreach (var (key, value) in extra ?? [])
        {
            data[key] = value;
        }

        return _mailer.SendMail(new MailAddress { Email = email, Name = name }, new MailMessage
        {
            From = new MailAddress { Email = _mailConfig.PostMasterEmail, Name = "simpled" },
            TemplateLang = Language,
            TemplateName = templateName,
            TemplateData = data,
        });
    }
}
