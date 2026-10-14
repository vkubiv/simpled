using SendGrid;
using SendGrid.Helpers.Mail;

namespace core.Services.Mailer;

public class RealMockCombinedMailer : IMailer
{
    private readonly IMailer _realMailer;
    private readonly IMailer _mockMailer;

    public RealMockCombinedMailer(IMailer realMailer, IMailer mockMailer)
    {
        _realMailer = realMailer;
        _mockMailer = mockMailer;
    }

    public async Task SendMail(MailAddress toEmail, MailMessage message)
    {
        if (toEmail.Email.EndsWith("@test.com", StringComparison.InvariantCultureIgnoreCase))
        {
            await _mockMailer.SendMail(toEmail, message);
        }
        else
        {
            await _realMailer.SendMail(toEmail, message);
        }
    }
}