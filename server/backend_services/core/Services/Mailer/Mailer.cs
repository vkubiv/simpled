namespace core.Services.Mailer;

public class MailAddress
{
    public string? Name { get; set; }

    public required string Email { get; set; }
}

public class MailMessage
{
    public required MailAddress From { get; set; }

    public required string TemplateLang { get; set; }
    public required string TemplateName { get; set; }
    
    public required Dictionary<String, object> TemplateData { get; set; }
}

public interface IMailer
{
    Task SendMail(MailAddress toEmail, MailMessage message);
}