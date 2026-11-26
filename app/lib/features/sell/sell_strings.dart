import '../../core/api/api_error.dart';
import '../../core/strings.dart';

/// The copy of the selling area. Widgets take their words from here.
abstract final class SellStrings {
  // Entry.
  static const String sellEntry = 'Sell on Abuja Market';
  static const String myStores = 'My stores';
  static const String sellArea = 'Selling';
  static const String backToShopping = 'Back to shopping';
  static const String switchStore = 'Switch store';

  // Access code.
  static const String codeTitle = 'Become a seller';
  static const String codeBody =
      'Enter the access code you were given. Codes are issued to sellers we have chosen.';
  static const String codeLabel = 'Access code';
  static const String codeContinue = 'Continue';
  static const String codeRefused =
      'That code did not work. Check it, or ask for a new one.';

  // Identity.
  static const String identityTitle = 'Confirm who you are';
  static const String noticeHeading = 'Before we start';
  static const String agree = 'I agree to this check';
  static const String ninLabel = 'National Identification Number (NIN)';
  static const String ninHelp = '11 digits.';
  static const String ninTooShort = 'A NIN has 11 digits.';
  static const String identityContinue = 'Continue';
  static const String takeSelfie = 'Take a selfie';
  static const String selfieHint =
      'You will be asked to look at the camera. It is sent straight to our identity partner.';
  static const String selfieCancelled =
      'The selfie was not completed. You can try again.';
  static const String selfieFailed =
      'The selfie could not be taken. Try again.';
  static const String waiting = 'Checking your details';
  static const String waitingBody =
      'This usually takes a minute. You can leave this screen: we will tell you the result.';
  static const String passedTitle = 'You are confirmed';
  static const String passedBody = 'Your stores can take orders.';
  static const String reviewTitle = 'We are looking at it';
  static const String reviewBody =
      'A person is checking your details. Nothing more to do for now. We will tell you the result.';
  static const String refusedTitle = 'We could not verify this NIN';
  static const String refusedBody =
      'Contact support if you think this is a mistake.';
  static const String revokedTitle = 'Your check was withdrawn';
  static const String revokedBody = 'Contact support to find out why.';
  static const String unavailableTitle = 'The check is not available right now';
  static const String unavailableBody = 'Please try again later.';
  static const String rateLimited =
      'Too many tries today. Please try again tomorrow.';
  static const String contactSupport = 'Contact support';
  static const String tryAgain = 'Try again';
  static const String done = 'Done';
  static String graceBanner(String date) =>
      'Confirm your identity by $date so your store can keep taking orders.';
  static const String identityBannerMember =
      'Confirm your identity to act on orders.';
  static const String identityBannerAction = 'Confirm now';

  // Store creation.
  static const String createStoreTitle = 'Open your store';
  static const String storeName = 'Store name';
  static const String storeDescription = 'What you sell';
  static const String districtTitle = 'Where in the market';
  static const String pickupAddress = 'Pickup address';
  static const String areaCouncil = 'Area council';
  static const String businessPhone = 'Business phone';
  static const String deliveryTitle = 'How you deliver';
  static const String courierDelivery = 'Courier picks up from you';
  static const String selfDelivery = 'I deliver myself';
  static const String selfDeliveryFee = 'Delivery fee (naira)';
  static const String createStore = 'Open store';
  static const String storeNeedsDetails = 'Fill in every field to continue.';
  static const String pickupNote =
      'This becomes your first branch. You can change it later under Branches.';

  // Payout.
  static const String payoutTitle = 'Where you get paid';
  static const String bank = 'Bank';
  static const String accountNumber = 'Account number (10 digits)';
  static const String accountNumberShort = 'An account number has 10 digits.';
  static const String password = 'Your password';
  static const String passwordNeeded =
      'Enter your password to change the account.';
  static const String savePayout = 'Save account';
  static const String payoutVerified = 'Verified';
  static const String payoutPending = 'Waiting for a check';
  static const String payoutNameMismatch =
      'The name on this account does not match your name. We will look at it.';
  static const String payoutWaitingBoth =
      'Your store opens once the account is verified and your identity is confirmed.';
  static String payoutLine(String last4, String? name) =>
      name == null ? 'Account ending $last4' : '$name, ending $last4';

  // Tabs.
  static const String tabOrders = 'Orders';
  static const String tabProducts = 'Products';
  static const String tabStore = 'Store';
  static const String tabTeam = 'Team';

  // Orders.
  static const String ordersToShip = 'To ship';
  static const String ordersOnTheWay = 'On the way';
  static const String ordersDone = 'Done';
  static const String noOrders = 'No orders here';
  static const String noOrdersBody =
      'New orders show up here as soon as they are paid.';
  static const String loadMore = 'Load more';
  static const String stepReady = 'Mark ready';
  static const String stepPickedUp = 'Picked up';
  static const String stepOut = 'Out for delivery';
  static const String stepDelivered = 'Delivered, enter buyer code';
  static const String stepCollected = 'Handed over, enter buyer code';
  static const String codeEntry = 'Buyer handover code';
  static const String codeEntryHelp = 'Ask the buyer for their 6-digit code.';
  static const String codeWrong = 'That code is not right.';
  static const String codeLocked = 'Too many wrong codes. Try again later.';
  static const String orderHeld =
      'This order is on hold. Nothing to do until it is settled.';
  static const String shipBy = 'Ship by';
  static const String payout = 'You receive';
  static const String branchLabel = 'Branch';
  static String methodLabel(String m) => switch (m) {
    'courier' => 'Courier',
    'self' => 'You deliver',
    'collect' => 'Buyer collects',
    _ => '',
  };
  static const String disputeOpen = 'The buyer reported a problem';

  // Products.
  static const String noProducts = 'No products yet';
  static const String noProductsBody =
      'Add your first product with a few good photos.';
  static const String addProduct = 'Add product';
  static const String editProduct = 'Edit product';
  static const String productName = 'Product name';
  static const String productDescription = 'Description';
  static const String productCategory = 'Category';
  static const String productCondition = 'Condition';
  static const String productPrice = 'Price (naira)';
  static const String productVariants = 'Sizes and stock';
  static const String variantLabel = 'Size or option (optional)';
  static const String variantStock = 'In stock';
  static const String addVariant = 'Add another';
  static const String publish = 'Publish';
  static const String unpublish = 'Hide from buyers';
  static const String saveProduct = 'Save product';
  static const String published = 'Live';
  static const String hidden = 'Hidden';
  static const String deleteProduct = 'Delete product';
  static String stockLine(int n) => '$n in stock';
  static const String conditionNew = 'New';
  static const String conditionUsed = 'Used';
  static const String productNeedsDetails =
      'Add a name, a price and at least one photo.';

  // Photos.
  static const String photosTitle = 'Photos';
  static const String photosHelp =
      'The first photo is the cover. Square photos in good light sell best.';
  static const String takePhoto = 'Take a photo';
  static const String fromGallery = 'Choose from gallery';
  static const String guideTitle = 'Photos that sell';
  static const List<String> guideTips = [
    'Use a plain background and window light, not behind the product.',
    'Fill the frame. One product per photo.',
    'Take the front, the back, a close detail and one in use.',
    'Keep faces and house numbers out of the picture.',
  ];
  static const String guideGotIt = 'Got it';
  static const List<String> shotTips = ['Front', 'Back', 'Detail', 'In use'];
  static const String shutter = 'Take photo';
  static const String flash = 'Flash';
  static const String cameraDenied =
      'The camera is off for this app. Choose from your gallery instead, or turn it on in settings.';
  static const String uploading = 'Uploading';
  static const String uploadFailed = 'Not uploaded';
  static const String removePhoto = 'Remove photo';
  static const String retryPhoto = 'Try again';
  static const String maxPhotos = 'That is the most photos a product can have.';

  // Store tab.
  static const String storeDetails = 'Details';
  static const String storeStatusPending = 'Not open yet';
  static const String storeStatusActive = 'Open';
  static const String storeStatusSuspended = 'Suspended';
  static const String walkIn = 'Walk into my store';
  static const String branchesTitle = 'Branches';
  static const String branchesRow = 'Branches and pickup';
  static const String payoutRow = 'Payout account';
  static const String reviewsRow = 'Reviews';
  static const String disputesRow = 'Reported problems';
  static const String saveChanges = 'Save changes';
  static const String collectSwitch = 'Buyers can collect here';
  static const String collectNotice =
      'Buyers will see this branch\'s address and business phone after they pay.';
  static const String collectConfirm = 'Turn on collect';
  static const String cancel = 'Cancel';
  static const String addBranch = 'Add a branch';
  static const String branchName = 'Branch name';
  static const String closeBranch = 'Close this branch';
  static const String closeBranchBody =
      'Its stock goes. You can only close a branch with no open orders.';
  static const String closed = 'Closed';
  static const String branchSaved = 'Saved.';

  // Team.
  static const String teamTitle = 'Your team';
  static const String inviteTitle = 'Invite someone';
  static const String invitePhone = 'Their phone number';
  static const String roleManager = 'Manager';
  static const String roleStaff = 'Staff';
  static const String roleOwner = 'Owner';
  static const String roleLabel = 'Role';
  static const String tiedTo = 'Works at';
  static const String allBranches = 'All branches';
  static const String sendInvite = 'Send invite';
  static const String pendingInvites = 'Waiting for an answer';
  static const String remove = 'Remove';
  static const String revoke = 'Cancel invite';
  static const String noTeam = 'Just you so far';
  static const String managerNote =
      'Managers work across the store. Staff can be tied to one branch.';
  static String inviteLine(String last4) => 'Phone ending $last4';

  // Invites inbox.
  static const String invitesTitle = 'Invitations';
  static const String accept = 'Accept';
  static const String decline = 'Decline';
  static String inviteFrom(String store, String role) =>
      '$store invited you as $role';

  // Reviews and disputes.
  static const String reviewsTitle = 'Reviews';
  static const String noReviews = 'No reviews yet';
  static const String reply = 'Reply';
  static const String replyLabel = 'Your reply';
  static const String sendReply = 'Send reply';
  static const String replied = 'Your reply';
  static const String disputesTitle = 'Reported problems';
  static const String noDisputes = 'Nothing reported';
  static const String buyerSaid = 'The buyer said';
  static const String yourAnswer = 'Your answer';
  static const String answerLabel = 'Your side of it';
  static const String sendAnswer = 'Send answer';
  static const String answerBy = 'Answer by';
  static const String returnReceived = 'I received the return';
  static const String disputeNeedsAnswer = 'Needs your answer';
  static const String disputeNeedsText = 'Write your answer first.';

  // Errors.
  static const String notAllowed = 'Your role cannot do that.';
  static const String identityRequired =
      'Confirm your identity first, then try again.';
  static const String storeLimit = 'You have reached the limit for stores.';
  static const String saveFailed =
      'That could not be saved. Check the details and try again.';
}

/// Plain words for a failed call in the selling area.
String sellError(Object error) {
  if (error is! ApiError) return Strings.genericError;
  if (error.isOffline) return Strings.offlineMessage;
  if (error.isTimeout) return Strings.timeoutMessage;
  switch (error.code) {
    // A message written in the app (the selfie result), safe to show.
    case 'selfie':
      return error.message;
    case 'not_allowed':
      return SellStrings.notAllowed;
    case 'identity_required':
      return SellStrings.identityRequired;
    case 'identity_rate_limited':
      return SellStrings.rateLimited;
    case 'identity_not_accepted':
      return SellStrings.refusedTitle;
    case 'identity_unavailable':
      return SellStrings.unavailableBody;
    case 'identity_in_review':
      return SellStrings.reviewBody;
    case 'handover_code_wrong':
      return SellStrings.codeWrong;
    case 'handover_locked':
      return SellStrings.codeLocked;
    case 'code_invalid':
    case 'code_expired':
    case 'code_exhausted':
    case 'code_revoked':
      return SellStrings.codeRefused;
    case 'validation_failed':
    case 'invalid_phone':
      return SellStrings.saveFailed;
  }
  if (error.isServerDown) return Strings.serverDownMessage;
  return Strings.genericError;
}
