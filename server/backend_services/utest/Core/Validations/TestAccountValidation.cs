using core.Libs.MVCValidation;
using core.Validations;
using utest.TestSupport;

namespace utest.Core.Validations;

public class TestAccountValidation
{
    [Theory]
    [InlineData("ann@test.com", true)]
    [InlineData(" ann@test.com ", true)]
    [InlineData("Ann <ann@test.com>", false)]
    [InlineData("not-an-email", false)]
    public void TestValidateEmail(string email, bool valid)
    {
        var vr = new ValidationResult();
        Assert.Equal(valid, AccountValidation.ValidateEmail(email, vr));
        if (!valid)
        {
            VRAssert.HasError(vr, ErrorCode.IncorrectValue, "email");
            vr.ReturnErrorResponse();
        }
    }

    [Fact]
    public void TestValidatePassword_NotTheEmail()
    {
        var vr = new ValidationResult();
        Assert.False(AccountValidation.ValidatePassword("Ann@Test.com", "ann@test.com", vr));
        VRAssert.HasError(vr, ErrorCode.IncorrectValue, "password");
        vr.ReturnErrorResponse();
    }
}
