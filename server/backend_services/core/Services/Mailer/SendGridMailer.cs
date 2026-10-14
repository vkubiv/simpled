using SendGrid;
using SendGrid.Helpers.Mail;

namespace core.Services.Mailer;

public class SendGridMailerOpts
{
    public required string ApiKey { get; set; }
    public string? Host { get; set; }
}

public class SendGridMailer : IMailer
{
    private readonly SendGridMailerOpts _opts;
    private readonly MainTemplater _templater;

    public SendGridMailer(SendGridMailerOpts opts, MainTemplater templater)
    {
        _opts = opts;
        _templater = templater;
    }

    public async Task SendMail(MailAddress toEmail, MailMessage message)
    {
        var client = new SendGridClient(_opts.ApiKey, _opts.Host);

        var mailContent =
            _templater.RenderMailContent(message.TemplateLang, message.TemplateName, message.TemplateData);

        var msg = new SendGridMessage
        {
            From = new EmailAddress(message.From.Email, message.From.Name),
            Subject = mailContent.Subject,
            HtmlContent = mailContent.Body,
        };

        msg.AddTo(new EmailAddress(toEmail.Email, toEmail.Name));

        await client.SendEmailAsync(msg);
    }
}