using core.Services.Mailer;

namespace utest.TestSupport.TestDoubles;

public class RecordingMailer : IMailer
{
    public List<(MailAddress To, MailMessage Message)> Sent { get; } = new();

    public Task SendMail(MailAddress toEmail, MailMessage message)
    {
        Sent.Add((toEmail, message));
        return Task.CompletedTask;
    }

    public UserMailer UserMailer() =>
        new(this, new UserMailConfig { PostMasterEmail = "no-reply@test.com", WebAppUrl = "http://app.test/" });

    /// <summary>The token in the last link mailed to <paramref name="email"/> with <paramref name="template"/>.</summary>
    public string LastToken(string email, string template)
    {
        var (_, message) = Sent.Last(m => m.To.Email == email && m.Message.TemplateName == template);
        var link = (string)message.TemplateData["link"];
        return Uri.UnescapeDataString(link[(link.IndexOf("token=", StringComparison.Ordinal) + "token=".Length)..]);
    }
}
