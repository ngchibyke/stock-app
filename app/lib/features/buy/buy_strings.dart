import '../../core/api/api_error.dart';
import '../../core/strings.dart';

/// The copy of the buying screens.
abstract final class BuyStrings {
  static const String checkout = 'Checkout';
  static const String chooseAddress = 'Where should it go?';
  static const String addAddress = 'Add an address';
  static const String noAddresses = 'You have no saved address yet.';
  static const String deliveryTitle = 'How it gets to you';
  static const String loadingOptions = 'Finding delivery options';
  static const String optionsFailed = 'We could not find delivery options.';
  static String sentFrom(String branch) => 'Sent from $branch';
  static const String splitNote = 'Your goods come from more than one branch.';
  static const String courier = 'Courier';
  static const String selfDelivery = 'Delivered by the seller';
  static const String collect = 'Collect from the branch';
  static const String collectNote =
      'After you pay you will see the branch address and phone, and a code to show at the counter.';
  static String promised(String date) => 'By $date';
  static const String free = 'Free';
  static String onlyAvailable(int n) => 'Only $n available';
  static const String changeCart = 'Change your cart';
  static const String nothingToSend =
      'Nothing from this store can be sent to this address.';
  static const String reviewTitle = 'Review and pay';
  static const String itemsTotal = 'Goods';
  static const String delivery = 'Delivery';
  static const String total = 'Total';
  static const String pay = 'Pay now';
  static const String chooseAll = 'Choose a way to send each part.';
  static const String methodMixed =
      'Pick the same kind of delivery for every part of this order.';
  static const String optionsChanged =
      'Stock or prices changed. We refreshed your options. Nothing was charged.';
  static const String tooManyPending =
      'You have payments waiting. Finish or let them expire first.';
  static const String cartEmpty = 'Your cart is empty.';
  static const String busy = 'The shop is busy. Try again in a moment.';
  static const String payFailed =
      'We could not start the payment. Nothing was charged. Try again.';

  // Payment.
  static const String paymentTitle = 'Pay';
  static const String closePayment = 'I am done';
  static const String paymentBlocked = 'That page is not allowed here.';
  static const String checking = 'Confirming your payment';
  static const String checkingBody =
      'We are asking the bank. Please keep this screen open.';
  static const String paidTitle = 'Payment received';
  static const String paidBody =
      'The seller has your order. The money is held until you confirm it arrived.';
  static const String pendingTitle = 'Still waiting for the bank';
  static const String pendingBody =
      'If you were charged, your order will appear in Orders. We will tell you.';
  static const String failedTitle = 'Payment did not go through';
  static const String failedBody =
      'You were not charged. Your cart is as you left it.';
  static const String viewOrders = 'View my orders';
  static const String backToCart = 'Back to cart';

  // Orders.
  static const String ordersTitle = 'My orders';
  static const String noOrders = 'No orders yet';
  static const String noOrdersBody = 'Things you buy show up here.';
  static const String loadMore = 'Load more';
  static const String orderTitle = 'Order';
  static const String trackingTitle = 'Where it is';
  static String statusLabel(String s) => switch (s) {
    'paid' => 'Paid, waiting to ship',
    'picked_up' => 'Picked up',
    'out_for_delivery' => 'Out for delivery',
    'delivered' => 'Delivered',
    'completed' => 'Completed',
    'cancelled' => 'Cancelled',
    'refund_pending' => 'Refund on the way',
    'refunded' => 'Refunded',
    _ => 'Waiting for payment',
  };
  static const String handoverTitle = 'Your handover code';
  static const String handoverHint =
      'Give this code only when you are holding your goods.';
  static const String pickupTitle = 'Pick it up here';
  static const String received = 'I received it';
  static const String receivedBody =
      'Confirm only if you hold everything. The seller is paid after this.';
  static const String cancel = 'Cancel order';
  static const String cancelBody =
      'You will get a full refund. Banks can take up to 10 working days.';
  static const String keep = 'Keep order';
  static const String confirm = 'Confirm';
  static const String reportProblem = 'Report a problem';
  static const String seeReport = 'See your report';
  static const String leaveReview = 'Leave a review';
  static const String openTracking = 'Open courier tracking';
  static const String refreshed = 'Updated';
  static String refunded(String amount) => 'Refunded $amount';

  // Dispute.
  static const String reportTitle = 'Report a problem';
  static const String reasonLabel = 'What went wrong?';
  static String reason(String wire) => switch (wire) {
    'not_received' => 'It did not arrive',
    'damaged' => 'It arrived damaged',
    'wrong_item' => 'Wrong item',
    'not_as_described' => 'Not as described',
    'incomplete' => 'Something is missing',
    _ => 'Something else',
  };
  static const String statementLabel = 'Tell us what happened';
  static const String sendReport = 'Send report';
  static const String reportPhotos = 'Photos (optional)';
  static const String reportStatusTitle = 'Your report';
  static const String awaitingSeller = 'Waiting for the seller to answer.';
  static const String awaitingReturn =
      'The market asked for the item to be sent back before deciding.';
  static const String awaitingDecision = 'The market is deciding.';
  static const String decided = 'Decided';
  static const String withdrawn = 'You withdrew this report.';
  static const String sellerAnswered = 'The seller answered.';
  static const String withdraw = 'Withdraw report';
  static const String withdrawBody =
      'You cannot report this order again after withdrawing.';

  // Review.
  static const String reviewScreen = 'Review this order';
  static const String starsLabel = 'Stars';
  static const String reviewText = 'Your review (optional)';
  static const String sendReview = 'Send review';
  static const String reviewSent = 'Thank you for your review.';

  // Inbox.
  static const String inboxTitle = 'Notices';
  static const String noNotices = 'No notices yet';
  static const String markAllRead = 'Mark all read';
  static const String pushTitle = 'Get order updates?';
  static const String pushBody =
      'We can send a notice when your order ships or the seller answers. Only a device token is sent to Google or Apple, never your order.';
  static const String pushYes = 'Yes, notify me';
  static const String pushNo = 'Not now';
  static const String myOrders = 'My orders';
}

/// Plain words for a failed call in the buying screens.
String buyError(Object error) {
  if (error is! ApiError) return Strings.genericError;
  if (error.isOffline) return Strings.offlineMessage;
  if (error.isTimeout) return Strings.timeoutMessage;
  switch (error.code) {
    case 'allocation_changed':
      return BuyStrings.optionsChanged;
    case 'too_many_pending_checkouts':
      return BuyStrings.tooManyPending;
    case 'cart_empty':
      return BuyStrings.cartEmpty;
    case 'checkout_busy':
      return BuyStrings.busy;
    case 'outside_service_area':
      return BuyStrings.nothingToSend;
  }
  if (error.status == 502) return BuyStrings.payFailed;
  if (error.isServerDown) return Strings.serverDownMessage;
  return Strings.genericError;
}
